pub mod recovery;
use crate::{
    binding::PaymentBindingClaim,
    pool::{NoteInput, NoteOutput, escrow::EscrowNote},
};
use zutti_protocol::{
    Digest, Error, Result, Secret32,
    encoding::{Canonical, Reader, Writer},
    order::OrderTerms,
    pool::{OperationKind, PoolClaim, PoolPath, nullifier_digest, output_digest},
};
use zutti_source::spv::{HeaderClaim, Policy, WindowClaim, WindowTerms};

pub const MAX_SPV_POOL_WITNESS: usize = 262_144;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpvPoolClaim {
    pub policy: Digest,
    pub binding_program: Digest,
    pub expiry: u32,
    pub pool: PoolClaim,
}
impl SpvPoolClaim {
    pub fn validate(&self) -> Result<()> {
        self.pool.validate()?;
        Digest::nonzero(self.policy.0)?;
        Digest::nonzero(self.binding_program.0)?;
        if (self.pool.kind == OperationKind::Reserve) != (self.expiry != 0)
            || self.expiry >= 500_000_000
        {
            return Err(Error::Invalid("SPV reservation expiry"));
        }
        Ok(())
    }
    pub fn journal_digest(&self) -> Result<Digest> {
        use sha2::Digest as _;
        Ok(Digest(sha2::Sha256::digest(self.to_bytes()?).into()))
    }
}
impl Canonical for SpvPoolClaim {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        self.validate()?;
        writer.fixed(b"ZUTTISPC");
        writer.u16(1);
        writer.fixed(&self.policy.0);
        writer.fixed(&self.binding_program.0);
        writer.u32(self.expiry);
        self.pool.encode(writer)
    }
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTISPC" || reader.u16()? != 1 {
            return Err(Error::Version);
        }
        let claim = Self {
            policy: Digest(reader.fixed()?),
            binding_program: Digest(reader.fixed()?),
            expiry: reader.u32()?,
            pool: PoolClaim::decode(reader)?,
        };
        claim.validate()?;
        Ok(claim)
    }
}

pub struct SpvReservation {
    pub terms: OrderTerms,
    pub order_randomness: Secret32,
    pub reservation_randomness: Secret32,
    pub txid: Digest,
    pub escrow: EscrowNote,
    pub nonce: [u8; 24],
    pub anchor: HeaderClaim,
    pub binding: PaymentBindingClaim,
    pub payout: NoteOutput,
    pub refund: NoteOutput,
}
impl core::fmt::Debug for SpvReservation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SpvReservation([REDACTED])")
    }
}
impl SpvReservation {
    pub fn window(&self) -> Result<WindowTerms> {
        WindowTerms::new(
            self.terms.domain().id()?,
            self.terms.commitment(&self.order_randomness)?,
            self.terms.window().anchor_height(),
            self.terms.window().anchor_hash(),
            self.terms.window().expiry(),
            self.terms.payout().commitment(),
            self.terms.refund().commitment(),
        )
    }
    pub fn binding(&self) -> Result<(Digest, Digest)> {
        use sha2::Digest as _;
        Ok((
            self.escrow.commitment()?,
            Digest(sha2::Sha256::digest(self.escrow.encrypt(self.nonce)?).into()),
        ))
    }
    pub fn validate(&self, policy: Policy, domain: Digest) -> Result<()> {
        self.binding
            .require_payment(
                policy,
                &self.terms,
                &self.order_randomness,
                &self.reservation_randomness,
                self.txid,
            )
            .map_err(|_| Error::Invalid("SPV prepared payment binding"))?;
        if self.terms.domain().id()? != domain
            || self.anchor.policy() != policy
            || self
                .anchor
                .height()
                .map_err(|_| Error::Invalid("SPV anchor height"))?
                != self.terms.window().anchor_height()
            || self
                .anchor
                .tip()
                .map_err(|_| Error::Invalid("SPV anchor hash"))?
                != self.terms.window().anchor_hash()
            || self.escrow.domain != domain
            || self.escrow.order != self.binding.order()
            || self.escrow.reservation != self.binding.reservation()
            || self.escrow.amount != self.terms.usdc().units()
            || self.payout.note.domain() != domain
            || self.refund.note.domain() != domain
            || self.payout.fixed()? != self.terms.payout()
            || self.refund.fixed()? != self.terms.refund()
        {
            return Err(Error::Invalid("SPV conditional reservation"));
        }
        Ok(())
    }
}
impl Canonical for SpvReservation {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        self.terms.encode(writer)?;
        writer.fixed(self.order_randomness.expose());
        writer.fixed(self.reservation_randomness.expose());
        writer.fixed(&self.txid.0);
        self.escrow.encode(writer)?;
        writer.fixed(&self.nonce);
        self.anchor.encode(writer)?;
        self.binding.encode(writer)?;
        self.payout.encode(writer)?;
        self.refund.encode(writer)
    }
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            terms: OrderTerms::decode(reader)?,
            order_randomness: Secret32::from_bytes(reader.fixed()?)?,
            reservation_randomness: Secret32::from_bytes(reader.fixed()?)?,
            txid: Digest::nonzero(reader.fixed()?)?,
            escrow: EscrowNote::decode(reader)?,
            nonce: reader.fixed()?,
            anchor: HeaderClaim::decode(reader)?,
            binding: PaymentBindingClaim::decode(reader)?,
            payout: NoteOutput::decode(reader)?,
            refund: NoteOutput::decode(reader)?,
        })
    }
}

#[derive(Debug)]
pub enum SpvAction {
    Deposit,
    Transfer,
    Withdraw,
    Reserve(Box<SpvReservation>),
    Settle {
        reservation: Box<SpvReservation>,
        path: Box<PoolPath>,
        window: Box<WindowClaim>,
    },
}
impl SpvAction {
    fn kind(&self) -> OperationKind {
        match self {
            Self::Deposit => OperationKind::Deposit,
            Self::Transfer => OperationKind::Transfer,
            Self::Withdraw => OperationKind::Withdraw,
            Self::Reserve(_) => OperationKind::Reserve,
            Self::Settle { .. } => OperationKind::Settle,
        }
    }
}

pub struct SpvPoolWitness {
    pub policy: Policy,
    pub claim: SpvPoolClaim,
    pub inputs: Vec<NoteInput>,
    pub outputs: Vec<NoteOutput>,
    pub action: SpvAction,
}
impl core::fmt::Debug for SpvPoolWitness {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SpvPoolWitness([REDACTED])")
    }
}
impl SpvPoolWitness {
    pub fn assumptions(&self) -> Result<Vec<(Digest, Vec<u8>)>> {
        self.evaluate()?;
        Ok(match &self.action {
            SpvAction::Reserve(reservation) => vec![
                (self.claim.binding_program, reservation.binding.to_bytes()?),
                (self.policy.program(), reservation.anchor.to_bytes()?),
            ],
            SpvAction::Settle {
                reservation,
                window,
                ..
            } => vec![
                (self.claim.binding_program, reservation.binding.to_bytes()?),
                (self.policy.program(), window.to_bytes()?),
            ],
            _ => vec![],
        })
    }
    pub fn evaluate(&self) -> Result<SpvPoolClaim> {
        self.claim.validate()?;
        let claim = &self.claim.pool;
        if self.claim.policy != self.policy.id()?
            || claim.kind != self.action.kind()
            || self.inputs.len() > 8
            || self.outputs.len() > 8
        {
            return Err(Error::Invalid("SPV pool operation shape"));
        }
        let mut nullifiers = Vec::new();
        let mut outputs = Vec::new();
        let mut input_value = 0_u128;
        let mut output_value = 0_u128;
        for input in &self.inputs {
            if input.note.domain() != claim.domain {
                return Err(Error::Invalid("SPV input domain"));
            }
            input
                .path
                .verify(claim.domain, input.note.commitment()?, claim.root)?;
            input
                .note
                .verify_spend(self.claim.journal_digest()?, &input.authorization)?;
            nullifiers.push(input.note.nullifier(&input.nullifier_key)?);
            input_value = input_value
                .checked_add(input.note.amount().units())
                .ok_or(Error::Overflow)?;
        }
        for output in &self.outputs {
            if output.note.domain() != claim.domain {
                return Err(Error::Invalid("SPV output domain"));
            }
            outputs.push(output.binding()?);
            output_value = output_value
                .checked_add(output.note.amount().units())
                .ok_or(Error::Overflow)?;
        }
        match &self.action {
            SpvAction::Deposit => {
                if !self.inputs.is_empty() || self.outputs.len() != 1 {
                    return Err(Error::Invalid("SPV deposit shape"));
                }
                input_value = u128::from(claim.value);
            }
            SpvAction::Transfer => {
                if self.inputs.is_empty() || self.outputs.is_empty() {
                    return Err(Error::Invalid("SPV transfer shape"));
                }
            }
            SpvAction::Withdraw => {
                if self.inputs.is_empty() {
                    return Err(Error::Invalid("SPV withdrawal shape"));
                }
                output_value = output_value
                    .checked_add(u128::from(claim.value))
                    .ok_or(Error::Overflow)?;
            }
            SpvAction::Reserve(reservation) => {
                if self.inputs.is_empty() || self.outputs.len() > 7 {
                    return Err(Error::Invalid("SPV reservation shape"));
                }
                reservation.validate(self.policy, claim.domain)?;
                if claim.checkpoint != reservation.anchor.id()?
                    || claim.order_tag != reservation.terms.use_tag()?
                    || claim.order != reservation.binding.order()
                    || claim.reservation != reservation.binding.reservation()
                    || self.claim.expiry != reservation.terms.window().expiry()
                {
                    return Err(Error::Invalid("SPV funding context"));
                }
                outputs.insert(0, reservation.binding()?);
                output_value = output_value
                    .checked_add(reservation.escrow.amount)
                    .ok_or(Error::Overflow)?;
            }
            SpvAction::Settle {
                reservation,
                path,
                window,
            } => {
                if !self.inputs.is_empty() || self.outputs.len() != 1 {
                    return Err(Error::Invalid("SPV settlement shape"));
                }
                reservation.validate(self.policy, claim.domain)?;
                reservation
                    .binding
                    .require_window(&reservation.window()?, window)
                    .map_err(|_| Error::Invalid("SPV window composition"))?;
                if window.prefix != reservation.anchor.id()?
                    || window.checkpoint != claim.checkpoint
                {
                    return Err(Error::Invalid("SPV terminal ancestry"));
                }
                path.verify(claim.domain, reservation.escrow.commitment()?, claim.root)?;
                let fixed = self.outputs[0].fixed()?;
                let expected = if window.output == reservation.terms.payout().commitment() {
                    reservation.terms.payout()
                } else {
                    reservation.terms.refund()
                };
                if fixed != expected
                    || fixed.commitment() != window.output
                    || self.outputs[0].note.amount().units() != reservation.escrow.amount
                {
                    return Err(Error::Invalid("SPV fixed settlement output"));
                }
                nullifiers.push(reservation.escrow.nullifier()?);
                input_value = reservation.escrow.amount;
            }
        }
        if input_value != output_value
            || nullifier_digest(&nullifiers)? != claim.nullifiers
            || output_digest(&outputs)? != claim.outputs
        {
            return Err(Error::Invalid("SPV pool conservation or public binding"));
        }
        Ok(self.claim.clone())
    }
}
impl Canonical for SpvPoolWitness {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        if self.inputs.len() > 8 || self.outputs.len() > 8 {
            return Err(Error::Length);
        }
        writer.fixed(b"ZUTTISPV");
        writer.u16(1);
        self.policy.encode(writer)?;
        self.claim.encode(writer)?;
        writer.u8(self.inputs.len() as u8);
        for input in &self.inputs {
            input.encode(writer)?;
        }
        writer.u8(self.outputs.len() as u8);
        for output in &self.outputs {
            output.encode(writer)?;
        }
        writer.u8(self.action.kind() as u8);
        match &self.action {
            SpvAction::Reserve(reservation) => reservation.encode(writer)?,
            SpvAction::Settle {
                reservation,
                path,
                window,
            } => {
                reservation.encode(writer)?;
                path.encode(writer)?;
                window.encode(writer)?;
            }
            _ => {}
        }
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTISPV" || reader.u16()? != 1 {
            return Err(Error::Version);
        }
        let policy = Policy::decode(reader)?;
        let claim = SpvPoolClaim::decode(reader)?;
        let count = reader.u8()?;
        if count > 8 {
            return Err(Error::Length);
        }
        let mut inputs = Vec::new();
        for _ in 0..count {
            inputs.push(NoteInput::decode(reader)?);
        }
        let count = reader.u8()?;
        if count > 8 {
            return Err(Error::Length);
        }
        let mut outputs = Vec::new();
        for _ in 0..count {
            outputs.push(NoteOutput::decode(reader)?);
        }
        let action = match OperationKind::from_id(reader.u8()?)? {
            OperationKind::Deposit => SpvAction::Deposit,
            OperationKind::Transfer => SpvAction::Transfer,
            OperationKind::Withdraw => SpvAction::Withdraw,
            OperationKind::Reserve => SpvAction::Reserve(Box::new(SpvReservation::decode(reader)?)),
            OperationKind::Settle => SpvAction::Settle {
                reservation: Box::new(SpvReservation::decode(reader)?),
                path: Box::new(PoolPath::decode(reader)?),
                window: Box::new(WindowClaim::decode(reader)?),
            },
        };
        Ok(Self {
            policy,
            claim,
            inputs,
            outputs,
            action,
        })
    }
    fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.is_empty() || bytes.len() > MAX_SPV_POOL_WITNESS {
            return Err(Error::Length);
        }
        let mut reader = Reader::new(bytes);
        let witness = Self::decode(&mut reader)?;
        reader.finish()?;
        Ok(witness)
    }
}
