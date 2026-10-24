use super::{MAX_SPV_POOL_WITNESS, SpvAction, SpvPoolClaim, SpvPoolWitness, SpvReservation};
use crate::pool::NoteOutput;
use zutti_protocol::{
    Digest, Error, Result,
    commitment::hash,
    encoding::{Canonical, Reader, Writer},
    pool::{OperationKind, PoolClaim, PoolPath, nullifier_digest, output_digest},
};
use zutti_source::spv::{Policy, WindowClaim};

pub struct SpvSwapRecord {
    policy: Policy,
    funding: SpvPoolClaim,
    outputs: Vec<(Digest, Digest)>,
    reservation: SpvReservation,
    funding_seal: Option<Vec<u8>>,
}
impl core::fmt::Debug for SpvSwapRecord {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SpvSwapRecord([REDACTED])")
    }
}
impl SpvSwapRecord {
    pub fn from_reservation(witness: &SpvPoolWitness) -> Result<Self> {
        let funding = witness.evaluate()?;
        let SpvAction::Reserve(reservation) = &witness.action else {
            return Err(Error::Invalid("expected SPV reservation"));
        };
        let mut outputs = vec![reservation.binding()?];
        for output in &witness.outputs {
            outputs.push(output.binding()?);
        }
        let record = Self {
            policy: witness.policy,
            funding,
            funding_seal: None,
            outputs,
            reservation: SpvReservation::from_bytes(&reservation.to_bytes()?)?,
        };
        record.validate()?;
        Ok(record)
    }
    fn validate(&self) -> Result<()> {
        self.funding.validate()?;
        if self
            .funding_seal
            .as_ref()
            .is_some_and(|seal| seal.len() != 260)
        {
            return Err(Error::Length);
        }
        self.reservation
            .validate(self.policy, self.funding.pool.domain)?;
        let claim = &self.funding.pool;
        if claim.kind != OperationKind::Reserve
            || self.funding.policy != self.policy.id()?
            || self.funding.expiry != self.reservation.terms.window().expiry()
            || claim.checkpoint != self.reservation.anchor.id()?
            || claim.order_tag != self.reservation.terms.use_tag()?
            || claim.order != self.reservation.binding.order()
            || claim.reservation != self.reservation.binding.reservation()
            || self.outputs.is_empty()
            || self.outputs.len() > 8
            || self.outputs[0] != self.reservation.binding()?
            || output_digest(&self.outputs)? != claim.outputs
        {
            return Err(Error::Invalid("SPV swap recovery record"));
        }
        Ok(())
    }
    pub fn id(&self) -> Result<Digest> {
        self.validate()?;
        Ok(hash(
            b"spv-swap-record",
            &[
                &self.funding.pool.domain.0,
                &self.funding.pool_program.0,
                &self.funding.pool.order.0,
                &self.funding.pool.reservation.0,
                &self.reservation.escrow.commitment()?.0,
            ],
        ))
    }
    pub fn domain(&self) -> Digest {
        self.funding.pool.domain
    }
    pub fn terms(&self) -> &zutti_protocol::order::OrderTerms {
        &self.reservation.terms
    }
    pub fn inspect_payment(&self, pczt: &[u8]) -> crate::Result<crate::prepare::InspectedPayment> {
        self.validate()?;
        let payment = crate::prepare::inspect_ironwood(
            pczt,
            &self.reservation.terms,
            &self.reservation.order_randomness,
        )?;
        if payment.txid() != self.reservation.txid {
            return Err(crate::Error::Payment);
        }
        Ok(payment)
    }
    pub fn with_funding_seal(mut self, seal: Vec<u8>) -> Result<Self> {
        if seal.len() != 260 {
            return Err(Error::Length);
        }
        self.funding_seal = Some(seal);
        self.validate()?;
        Ok(self)
    }
    pub fn funding_seal(&self) -> Option<&[u8]> {
        self.funding_seal.as_deref()
    }
    pub fn funding(&self) -> &SpvPoolClaim {
        &self.funding
    }
    pub fn reservation(&self) -> &SpvReservation {
        &self.reservation
    }
    pub fn settlement(
        &self,
        root: Digest,
        path: PoolPath,
        window: WindowClaim,
    ) -> Result<SpvPoolWitness> {
        self.validate()?;
        let selected = if window.output == self.reservation.terms.payout().commitment() {
            &self.reservation.payout
        } else if window.output == self.reservation.terms.refund().commitment() {
            &self.reservation.refund
        } else {
            return Err(Error::Invalid("unrecognized SPV recovery output"));
        };
        let output = NoteOutput::from_bytes(&selected.to_bytes()?)?;
        let claim = SpvPoolClaim {
            policy: self.funding.policy,
            binding_program: self.funding.binding_program,
            pool_program: self.funding.pool_program,
            expiry: 0,
            pool: PoolClaim {
                kind: OperationKind::Settle,
                domain: self.funding.pool.domain,
                root,
                checkpoint: window.checkpoint,
                order_tag: Digest::default(),
                order: Digest::default(),
                reservation: Digest::default(),
                value: 0,
                owner: [0; 20],
                nullifiers: nullifier_digest(&[self.reservation.escrow.nullifier()?])?,
                outputs: output_digest(&[output.binding()?])?,
            },
        };
        let witness = SpvPoolWitness {
            policy: self.policy,
            claim,
            inputs: vec![],
            outputs: vec![output],
            action: SpvAction::Settle {
                funding: Box::new(self.funding.clone()),
                funded_outputs: self.outputs.clone(),
                reservation: Box::new(SpvReservation::from_bytes(&self.reservation.to_bytes()?)?),
                path: Box::new(path),
                window: Box::new(window),
            },
        };
        witness.evaluate()?;
        Ok(witness)
    }
    #[cfg(feature = "native")]
    pub fn save_new(
        &self,
        path: &std::path::Path,
        key: &zutti_protocol::Secret32,
    ) -> crate::Result<()> {
        self.validate()?;
        let plaintext = zeroize::Zeroizing::new(self.to_bytes()?);
        crate::backup::persist_new(
            path,
            &crate::backup::seal(self.funding.pool.domain, &plaintext, key)?,
        )
    }
    #[cfg(feature = "native")]
    pub fn read(
        path: &std::path::Path,
        key: &zutti_protocol::Secret32,
        domain: Digest,
        expected: Digest,
    ) -> crate::Result<Self> {
        let archive = crate::backup::read_private(path, MAX_SPV_POOL_WITNESS + 1024)?;
        let record = Self::from_bytes(&crate::backup::open(domain, &archive, key)?)?;
        if record.funding.pool.domain != domain || record.id()? != expected {
            return Err(crate::Error::Archive);
        }
        Ok(record)
    }
}
impl Canonical for SpvSwapRecord {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        self.validate()?;
        writer.fixed(b"ZUTTISPR");
        writer.u16(2);
        self.policy.encode(writer)?;
        self.funding.encode(writer)?;
        writer.u8(self.outputs.len() as u8);
        for (commitment, ciphertext) in &self.outputs {
            writer.fixed(&commitment.0);
            writer.fixed(&ciphertext.0);
        }
        self.reservation.encode(writer)?;
        match &self.funding_seal {
            Some(seal) => {
                writer.u8(1);
                writer.bytes(seal)?;
            }
            None => writer.u8(0),
        }
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTISPR" || reader.u16()? != 2 {
            return Err(Error::Version);
        }
        let policy = Policy::decode(reader)?;
        let funding = SpvPoolClaim::decode(reader)?;
        let count = reader.u8()?;
        if count == 0 || count > 8 {
            return Err(Error::Length);
        }
        let mut outputs = Vec::new();
        for _ in 0..count {
            outputs.push((Digest(reader.fixed()?), Digest(reader.fixed()?)));
        }
        let reservation = SpvReservation::decode(reader)?;
        let funding_seal = match reader.u8()? {
            0 => None,
            1 => Some(reader.bytes(260)?.to_vec()),
            _ => return Err(Error::Version),
        };
        let record = Self {
            policy,
            funding,
            outputs,
            reservation,
            funding_seal,
        };
        record.validate()?;
        Ok(record)
    }
    fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.is_empty() || bytes.len() > MAX_SPV_POOL_WITNESS {
            return Err(Error::Length);
        }
        let mut reader = Reader::new(bytes);
        let record = Self::decode(&mut reader)?;
        reader.finish()?;
        Ok(record)
    }
}

#[derive(Debug, Clone)]
pub struct SpvSettlementEvidence {
    pub root: Digest,
    pub path: PoolPath,
    pub window: WindowClaim,
}
impl Canonical for SpvSettlementEvidence {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        Digest::nonzero(self.root.0)?;
        writer.fixed(b"ZUTTISPE");
        writer.u16(1);
        writer.fixed(&self.root.0);
        self.path.encode(writer)?;
        self.window.encode(writer)
    }
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTISPE" || reader.u16()? != 1 {
            return Err(Error::Version);
        }
        Ok(Self {
            root: Digest::nonzero(reader.fixed()?)?,
            path: PoolPath::decode(reader)?,
            window: WindowClaim::decode(reader)?,
        })
    }
    fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.is_empty() || bytes.len() > 4096 {
            return Err(Error::Length);
        }
        let mut reader = Reader::new(bytes);
        let evidence = Self::decode(&mut reader)?;
        reader.finish()?;
        Ok(evidence)
    }
}
