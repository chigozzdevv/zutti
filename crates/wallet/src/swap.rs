use crate::{
    Error, Result, backup,
    pool::{NoteOutput, PoolAction, PoolWitness, ReservationWitness, SettlementWitness},
};
use std::path::Path;
use zeroize::Zeroizing;
use zutti_protocol::{
    Digest, Secret32,
    commitment::hash,
    encoding::{Canonical, Reader, Writer},
    pool::{OperationKind, PoolClaim, PoolPath, nullifier_digest, output_digest},
};
use zutti_source::{
    IndexProof,
    checkpoint::{HeaderPath, SourceCheckpoint},
};

pub const MAX_SWAP_RECORD: usize = 131_072;
pub const MAX_SETTLEMENT_EVIDENCE: usize = 65_536;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettlementAction {
    Claim,
    Refund,
}

pub struct SwapRecord {
    funding: PoolClaim,
    outputs: Vec<(Digest, Digest)>,
    reservation: ReservationWitness,
}
impl core::fmt::Debug for SwapRecord {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SwapRecord([REDACTED])")
    }
}
impl SwapRecord {
    pub fn from_reservation(witness: &PoolWitness) -> Result<Self> {
        let funding = witness.evaluate()?;
        let PoolAction::Reserve(reservation) = &witness.action else {
            return Err(zutti_protocol::Error::Invalid("expected reservation").into());
        };
        let mut outputs = vec![reservation.binding()?];
        for output in &witness.outputs {
            outputs.push(output.binding()?);
        }
        let record = Self {
            funding,
            outputs,
            reservation: copy(reservation.as_ref())?,
        };
        record.validate()?;
        Ok(record)
    }
    fn validate(&self) -> Result<()> {
        self.funding.validate()?;
        self.reservation.verify(&self.funding)?;
        if self.funding.kind != OperationKind::Reserve
            || self.outputs.is_empty()
            || self.outputs.len() > 8
            || self.outputs[0] != self.reservation.binding()?
            || output_digest(&self.outputs)? != self.funding.outputs
        {
            return Err(zutti_protocol::Error::Invalid("swap reservation record").into());
        }
        Ok(())
    }
    pub fn id(&self) -> Result<Digest> {
        Ok(hash(
            b"swap-record",
            &[
                &self.funding.domain.0,
                &self.funding.order.0,
                &self.funding.reservation.0,
                &self.reservation.escrow.commitment()?.0,
            ],
        ))
    }
    pub const fn domain(&self) -> Digest {
        self.funding.domain
    }
    pub fn terms(&self) -> &zutti_protocol::order::OrderTerms {
        &self.reservation.terms
    }
    pub fn reservation_claim(&self) -> &PoolClaim {
        &self.funding
    }
    pub fn escrow_commitment(&self) -> Result<Digest> {
        Ok(self.reservation.escrow.commitment()?)
    }
    pub fn escrow_nullifier(&self) -> Result<Digest> {
        Ok(self.reservation.escrow.nullifier()?)
    }
    pub fn inspect_payment(&self, pczt: &[u8]) -> Result<crate::prepare::InspectedPayment> {
        self.validate()?;
        let payment = crate::prepare::inspect_ironwood(
            pczt,
            &self.reservation.terms,
            &self.reservation.order_randomness,
        )?;
        if payment.txid() != self.reservation.txid {
            return Err(Error::Payment);
        }
        Ok(payment)
    }
    pub fn save_new(&self, path: &Path, key: &Secret32) -> Result<()> {
        let plaintext = Zeroizing::new(self.to_bytes()?);
        let archive = backup::seal(self.domain(), &plaintext, key)?;
        backup::persist_new(path, &archive)
    }
    pub fn read(
        path: &Path,
        key: &Secret32,
        domain: Digest,
        expected_swap: Digest,
    ) -> Result<Self> {
        let archive = backup::read_private(path, MAX_SWAP_RECORD + 128)?;
        let plaintext = backup::open(domain, &archive, key)?;
        let record = Self::from_bytes(&plaintext)?;
        if record.domain() != domain || record.id()? != expected_swap {
            return Err(Error::Archive);
        }
        Ok(record)
    }
    pub fn settlement(
        &self,
        action: SettlementAction,
        evidence: &SettlementEvidence,
    ) -> Result<PoolWitness> {
        self.validate()?;
        if evidence.swap != self.id()? {
            return Err(zutti_protocol::Error::Invalid("settlement swap binding").into());
        }
        let output: NoteOutput = copy(match action {
            SettlementAction::Claim => &self.reservation.payout,
            SettlementAction::Refund => &self.reservation.refund,
        })?;
        let claim = PoolClaim {
            kind: OperationKind::Settle,
            domain: self.domain(),
            root: evidence.root,
            checkpoint: evidence.checkpoint.id()?,
            order_tag: Digest::default(),
            order: Digest::default(),
            reservation: Digest::default(),
            value: 0,
            owner: [0; 20],
            nullifiers: nullifier_digest(&[self.reservation.escrow.nullifier()?])?,
            outputs: output_digest(&[output.binding()?])?,
        };
        let witness = PoolWitness {
            claim,
            inputs: vec![],
            outputs: vec![output],
            action: PoolAction::Settle(Box::new(SettlementWitness {
                terms: self.reservation.terms.clone(),
                order_randomness: Secret32::from_bytes(
                    *self.reservation.order_randomness.expose(),
                )?,
                reservation_randomness: Secret32::from_bytes(
                    *self.reservation.reservation_randomness.expose(),
                )?,
                txid: self.reservation.txid,
                escrow: copy(&self.reservation.escrow)?,
                path: evidence.path.clone(),
                checkpoint: evidence.checkpoint.clone(),
                anchor: evidence.anchor.clone(),
                payment: evidence.payment.clone(),
            })),
        };
        witness.evaluate()?;
        Ok(witness)
    }
}
impl Canonical for SwapRecord {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        self.validate()
            .map_err(|_| zutti_protocol::Error::Invalid("swap reservation record"))?;
        writer.fixed(b"ZUTTISWP");
        writer.u16(1);
        self.funding.encode(writer)?;
        writer.u8(self.outputs.len() as u8);
        for (commitment, ciphertext) in &self.outputs {
            writer.fixed(&commitment.0);
            writer.fixed(&ciphertext.0);
        }
        self.reservation.encode(writer)
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTISWP" || reader.u16()? != 1 {
            return Err(zutti_protocol::Error::Version);
        }
        let funding = PoolClaim::decode(reader)?;
        let count = reader.u8()?;
        if count == 0 || count > 8 {
            return Err(zutti_protocol::Error::Length);
        }
        let mut outputs = Vec::with_capacity(usize::from(count));
        for _ in 0..count {
            outputs.push((Digest(reader.fixed()?), Digest(reader.fixed()?)));
        }
        let record = Self {
            funding,
            outputs,
            reservation: ReservationWitness::decode(reader)?,
        };
        record
            .validate()
            .map_err(|_| zutti_protocol::Error::Invalid("swap reservation record"))?;
        Ok(record)
    }
    fn from_bytes(bytes: &[u8]) -> zutti_protocol::Result<Self> {
        if bytes.len() > MAX_SWAP_RECORD {
            return Err(zutti_protocol::Error::Length);
        }
        let mut reader = Reader::new(bytes);
        let record = Self::decode(&mut reader)?;
        reader.finish()?;
        Ok(record)
    }
}

#[derive(Clone)]
pub struct SettlementEvidence {
    pub swap: Digest,
    pub root: Digest,
    pub path: PoolPath,
    pub checkpoint: SourceCheckpoint,
    pub anchor: HeaderPath,
    pub payment: IndexProof,
}
impl core::fmt::Debug for SettlementEvidence {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SettlementEvidence([REDACTED])")
    }
}
impl Canonical for SettlementEvidence {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        writer.fixed(b"ZUTTISWE");
        writer.u16(1);
        writer.fixed(&Digest::nonzero(self.swap.0)?.0);
        writer.fixed(&Digest::nonzero(self.root.0)?.0);
        self.path.encode(writer)?;
        self.checkpoint.encode(writer)?;
        self.anchor.encode(writer)?;
        self.payment.encode(writer)
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTISWE" || reader.u16()? != 1 {
            return Err(zutti_protocol::Error::Version);
        }
        Ok(Self {
            swap: Digest::nonzero(reader.fixed()?)?,
            root: Digest::nonzero(reader.fixed()?)?,
            path: PoolPath::decode(reader)?,
            checkpoint: SourceCheckpoint::decode(reader)?,
            anchor: HeaderPath::decode(reader)?,
            payment: IndexProof::decode(reader)?,
        })
    }
    fn from_bytes(bytes: &[u8]) -> zutti_protocol::Result<Self> {
        if bytes.len() > MAX_SETTLEMENT_EVIDENCE {
            return Err(zutti_protocol::Error::Length);
        }
        let mut reader = Reader::new(bytes);
        let evidence = Self::decode(&mut reader)?;
        reader.finish()?;
        Ok(evidence)
    }
}

fn copy<T: Canonical>(value: &T) -> zutti_protocol::Result<T> {
    T::from_bytes(&Zeroizing::new(value.to_bytes()?))
}
