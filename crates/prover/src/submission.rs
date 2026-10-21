use crate::{
    Error,
    artifact::{ProofArtifact, persist_bounded},
    pool::proof_kind,
    source::groth16_seal,
};
use sha2::{Digest as _, Sha256};
use std::path::Path;
use zutti_protocol::{
    Digest,
    encoding::Canonical,
    pool::{OperationKind, PoolClaim, nullifier_digest, output_digest},
    statement::{ProofKind, Statement},
};
use zutti_wallet::{
    pool::{PoolAction, PoolWitness},
    preparation::PreparationClaim,
};

#[derive(Debug, Clone)]
pub struct PublicOutput {
    pub commitment: Digest,
    pub ciphertext: Vec<u8>,
}

#[derive(Debug)]
pub struct PoolOperation {
    claim: PoolClaim,
    nullifiers: Vec<Digest>,
    outputs: Vec<PublicOutput>,
}
impl PoolOperation {
    pub fn new(
        claim: PoolClaim,
        nullifiers: Vec<Digest>,
        outputs: Vec<PublicOutput>,
    ) -> Result<Self, Error> {
        claim.validate()?;
        let shape = match claim.kind {
            OperationKind::Deposit => nullifiers.is_empty() && outputs.len() == 1,
            OperationKind::Transfer | OperationKind::Reserve => {
                !nullifiers.is_empty() && !outputs.is_empty()
            }
            OperationKind::Withdraw => !nullifiers.is_empty(),
            OperationKind::Settle => nullifiers.len() == 1 && outputs.len() == 1,
        };
        if !shape
            || outputs
                .iter()
                .any(|o| o.ciphertext.is_empty() || o.ciphertext.len() > 4096)
        {
            return Err(Error::Binding);
        }
        let bindings: Vec<_> = outputs
            .iter()
            .map(|output| {
                (
                    output.commitment,
                    Digest(Sha256::digest(&output.ciphertext).into()),
                )
            })
            .collect();
        if nullifier_digest(&nullifiers)? != claim.nullifiers
            || output_digest(&bindings)? != claim.outputs
        {
            return Err(Error::Binding);
        }
        Ok(Self {
            claim,
            nullifiers,
            outputs,
        })
    }
    pub fn from_witness(witness: &PoolWitness) -> Result<Self, Error> {
        let claim = witness.evaluate().map_err(|_| Error::Witness)?;
        let mut nullifiers = Vec::new();
        let mut outputs = Vec::new();
        for input in &witness.inputs {
            nullifiers.push(input.note.nullifier(&input.nullifier_key)?);
        }
        match &witness.action {
            PoolAction::Reserve(reservation) => outputs.push(PublicOutput {
                commitment: reservation.escrow.commitment()?,
                ciphertext: reservation.escrow.encrypt(reservation.nonce)?,
            }),
            PoolAction::Settle(settlement) => nullifiers.push(settlement.escrow.nullifier()?),
            _ => {}
        }
        for output in &witness.outputs {
            outputs.push(PublicOutput {
                commitment: output.note.commitment()?,
                ciphertext: output.encrypted()?.to_bytes()?,
            });
        }
        Self::new(claim, nullifiers, outputs)
    }
    pub const fn claim(&self) -> &PoolClaim {
        &self.claim
    }
}

#[derive(Debug)]
pub struct PoolExecution {
    operation: PoolOperation,
    image: Digest,
    seal: Vec<u8>,
    preparation_seal: Vec<u8>,
}
impl PoolExecution {
    pub fn verify(
        operation: PoolOperation,
        expected: &Statement,
        artifact: &ProofArtifact,
        preparation: Option<(&Statement, &ProofArtifact)>,
    ) -> Result<Self, Error> {
        if expected.kind() != proof_kind(operation.claim.kind)
            || expected.domain() != operation.claim.domain
            || expected.payload() != operation.claim.to_bytes()?
        {
            return Err(Error::Binding);
        }
        let preparation_seal = match (operation.claim.kind, preparation) {
            (OperationKind::Reserve, Some((statement, artifact))) => {
                let claim = PreparationClaim::from_bytes(statement.payload())?;
                if statement.kind() != ProofKind::Preparation
                    || statement.domain() != operation.claim.domain
                    || claim.domain() != operation.claim.domain
                    || claim.order() != operation.claim.order
                    || claim.reservation() != operation.claim.reservation
                {
                    return Err(Error::Binding);
                }
                groth16_seal(artifact, statement)?
            }
            (OperationKind::Reserve, None) | (_, Some(_)) => return Err(Error::Binding),
            (_, None) => Vec::new(),
        };
        let seal = groth16_seal(artifact, expected)?;
        Ok(Self {
            operation,
            image: expected.program(),
            seal,
            preparation_seal,
        })
    }
    pub fn to_json(&self) -> serde_json::Value {
        let op = &self.operation;
        let claim = &op.claim;
        serde_json::json!({
            "version": 1,
            "kind": claim.kind as u8,
            "domain": claim.domain.to_string(),
            "image_id": self.image.to_string(),
            "root": claim.root.to_string(),
            "checkpoint": claim.checkpoint.to_string(),
            "order_tag": claim.order_tag.to_string(),
            "order": claim.order.to_string(),
            "reservation": claim.reservation.to_string(),
            "value": claim.value,
            "owner": hex::encode(claim.owner),
            "nullifiers": op.nullifiers.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "outputs": op.outputs.iter().map(|output| serde_json::json!({
                "commitment": output.commitment.to_string(),
                "ciphertext": hex::encode(&output.ciphertext),
            })).collect::<Vec<_>>(),
            "seal": hex::encode(&self.seal),
            "preparation_seal": hex::encode(&self.preparation_seal),
            "proof_verified": true,
            "chain_admission_verified": false,
            "transaction_submitted": false,
        })
    }
    pub fn persist_new(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let mut bytes = serde_json::to_vec_pretty(&self.to_json())?;
        bytes.push(b'\n');
        persist_bounded(path, &bytes, 131_072)
    }
}
