use crate::{Error, ProofEnvelope, local::LocalProver};
use zeroize::Zeroizing;
use zutti_protocol::{
    Digest,
    encoding::Canonical,
    statement::{ProofKind, Statement},
};
use zutti_wallet::preparation::{MAX_PREPARATION_WITNESS, PreparationClaim, PreparationWitness};

pub fn preparation_image_id() -> Digest {
    let image = risc0_zkvm::sha::Digest::from(zutti_methods::PREPARATION_ID);
    Digest(image.into())
}

pub fn preparation_statement(claim: &PreparationClaim) -> Result<Statement, Error> {
    Ok(Statement::new(
        ProofKind::Preparation,
        claim.domain(),
        preparation_image_id(),
        claim.to_bytes()?,
    )?)
}

impl LocalProver {
    pub fn prove_preparation(
        &self,
        witness: &PreparationWitness,
    ) -> Result<(Statement, ProofEnvelope), Error> {
        let claim = witness.evaluate().map_err(|_| Error::Witness)?;
        let bytes = Zeroizing::new(witness.to_bytes()?);
        if bytes.is_empty() || bytes.len() > MAX_PREPARATION_WITNESS {
            return Err(Error::Witness);
        }
        self.prove(
            bytes,
            preparation_statement(&claim)?,
            zutti_methods::PREPARATION_ELF,
            268_435_456,
            20,
        )
    }
}
