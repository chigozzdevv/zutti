use crate::{
    pool::{NoteInput, NoteOutput},
    private_note::NoteKeys,
    spv_pool::{SpvAction, SpvPoolClaim, SpvPoolWitness, SpvReservation},
};
use zutti_protocol::{
    Digest, Error, Result,
    encoding::{Canonical, Reader, Writer},
    order::OrderTerms,
    pool::{OperationKind, PoolPath, nullifier_digest, output_digest},
    private_note::{NoteRecipient, PrivateNote},
};
use zutti_source::spv::Policy;

pub struct FundingInput {
    pub note: PrivateNote,
    pub path: PoolPath,
}

impl Canonical for SpvFundingPlan {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.fixed(b"ZUTTIFPL");
        writer.u16(1);
        self.policy.encode(writer)?;
        self.claim.encode(writer)?;
        writer.u8(u8::try_from(self.inputs.len()).map_err(|_| Error::Length)?);
        for input in &self.inputs {
            input.note.encode(writer)?;
            input.path.encode(writer)?;
        }
        writer.u8(u8::try_from(self.change.len()).map_err(|_| Error::Length)?);
        for output in &self.change {
            output.encode(writer)?;
        }
        self.reservation.encode(writer)
    }
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTIFPL" || reader.u16()? != 1 {
            return Err(Error::Version);
        }
        let policy = Policy::decode(reader)?;
        let claim = SpvPoolClaim::decode(reader)?;
        let count = reader.u8()?;
        if count == 0 || count > 8 {
            return Err(Error::Length);
        }
        let mut inputs = Vec::new();
        for _ in 0..count {
            inputs.push(FundingInput {
                note: PrivateNote::decode(reader)?,
                path: PoolPath::decode(reader)?,
            });
        }
        let count = reader.u8()?;
        if count > 7 {
            return Err(Error::Length);
        }
        let mut change = Vec::new();
        for _ in 0..count {
            change.push(NoteOutput::decode(reader)?);
        }
        Self::new(
            policy,
            claim,
            inputs,
            change,
            Box::new(SpvReservation::decode(reader)?),
        )
    }
}

pub struct FundingConsent<'a> {
    pub terms: &'a OrderTerms,
    pub refund: NoteRecipient,
    pub policy: Policy,
    pub binding_program: Digest,
    pub pool_program: Digest,
}

impl core::fmt::Debug for FundingInput {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("FundingInput([REDACTED])")
    }
}

impl core::fmt::Debug for FundingConsent<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("FundingConsent([REDACTED])")
    }
}

pub struct SpvFundingPlan {
    policy: Policy,
    claim: SpvPoolClaim,
    inputs: Vec<FundingInput>,
    change: Vec<NoteOutput>,
    reservation: Box<SpvReservation>,
}

impl core::fmt::Debug for SpvFundingPlan {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SpvFundingPlan([REDACTED])")
    }
}

impl SpvFundingPlan {
    pub fn from_inventory(
        policy: Policy,
        binding_program: Digest,
        pool_program: Digest,
        inventory: &crate::inventory::PoolInventory,
        keys: &NoteKeys,
        reservation: Box<SpvReservation>,
    ) -> Result<Self> {
        use crate::private_note::NoteEncryption;
        use zutti_protocol::{
            Secret32,
            amount::{Amount, Asset},
            pool::PoolClaim,
        };
        let domain = reservation.terms.domain().id()?;
        reservation.validate(policy, domain)?;
        let owner = keys.recipient()?;
        let value = reservation.terms.usdc().units();
        let coin = inventory
            .notes()
            .iter()
            .find(|owned| {
                owned.note.recipient() == owner
                    && owned.note.domain() == domain
                    && owned.note.amount().units() >= value
            })
            .ok_or(Error::Invalid("insufficient maker inventory"))?;
        let mut change = Vec::new();
        let remaining = coin
            .note
            .amount()
            .units()
            .checked_sub(value)
            .ok_or(Error::Overflow)?;
        if remaining > 0 {
            change.push(NoteOutput {
                note: PrivateNote::new(
                    domain,
                    owner,
                    Amount::from_units(Asset::Usdc, remaining)?,
                    Secret32::random()?,
                    Secret32::random()?,
                )?,
                encryption: NoteEncryption::random()?,
            });
        }
        let mut outputs = vec![reservation.binding()?];
        for output in &change {
            outputs.push(output.binding()?);
        }
        let claim = SpvPoolClaim {
            policy: policy.id()?,
            binding_program,
            pool_program,
            expiry: reservation.terms.window().expiry(),
            pool: PoolClaim {
                kind: OperationKind::Reserve,
                domain,
                root: inventory.root(),
                checkpoint: reservation.anchor.id()?,
                order_tag: reservation.terms.use_tag()?,
                order: reservation.binding.order(),
                reservation: reservation.binding.reservation(),
                value: 0,
                owner: [0; 20],
                nullifiers: nullifier_digest(&[keys.nullifier(&coin.note)?])?,
                outputs: output_digest(&outputs)?,
            },
        };
        let plan = Self::new(
            policy,
            claim,
            vec![FundingInput {
                note: PrivateNote::from_bytes(&coin.note.to_bytes()?)?,
                path: coin.path.clone(),
            }],
            change,
            reservation,
        )?;
        plan.check(
            &FundingConsent {
                terms: &plan.reservation.terms,
                refund: owner,
                policy,
                binding_program,
                pool_program,
            },
            keys,
        )?;
        Ok(plan)
    }
    pub fn new(
        policy: Policy,
        claim: SpvPoolClaim,
        inputs: Vec<FundingInput>,
        change: Vec<NoteOutput>,
        reservation: Box<SpvReservation>,
    ) -> Result<Self> {
        if inputs.is_empty()
            || inputs.len() > 8
            || change.len() > 7
            || claim.pool.kind != OperationKind::Reserve
        {
            return Err(Error::Invalid("funding plan shape"));
        }
        claim.validate()?;
        Ok(Self {
            policy,
            claim,
            inputs,
            change,
            reservation,
        })
    }

    pub const fn claim(&self) -> &SpvPoolClaim {
        &self.claim
    }

    pub fn reservation(&self) -> &SpvReservation {
        &self.reservation
    }

    pub fn nullifiers(&self, keys: &NoteKeys) -> Result<Vec<Digest>> {
        self.inputs
            .iter()
            .map(|input| keys.nullifier(&input.note))
            .collect()
    }

    pub fn check(&self, consent: &FundingConsent<'_>, keys: &NoteKeys) -> Result<()> {
        let owner = keys.recipient()?;
        let claim = &self.claim.pool;
        let reservation = &self.reservation;
        if consent.terms != &reservation.terms
            || consent.refund != owner
            || reservation.refund.note.recipient() != owner
            || consent.policy != self.policy
            || self.claim.policy != self.policy.id()?
            || self.claim.binding_program != consent.binding_program
            || self.claim.pool_program != consent.pool_program
        {
            return Err(Error::Invalid("funding consent mismatch"));
        }
        reservation.validate(self.policy, claim.domain)?;
        if claim.checkpoint != reservation.anchor.id()?
            || claim.order_tag != reservation.terms.use_tag()?
            || claim.order != reservation.binding.order()
            || claim.reservation != reservation.binding.reservation()
            || self.claim.expiry != reservation.terms.window().expiry()
        {
            return Err(Error::Invalid("funding reservation mismatch"));
        }
        let mut input_value = 0_u128;
        for input in &self.inputs {
            if input.note.domain() != claim.domain || input.note.recipient() != owner {
                return Err(Error::Invalid("funding input ownership"));
            }
            input
                .path
                .verify(claim.domain, input.note.commitment()?, claim.root)?;
            input_value = input_value
                .checked_add(input.note.amount().units())
                .ok_or(Error::Overflow)?;
        }
        let mut output_value = reservation.escrow.amount;
        let mut outputs = vec![reservation.binding()?];
        for output in &self.change {
            if output.note.domain() != claim.domain || output.note.recipient() != owner {
                return Err(Error::Invalid("funding change ownership"));
            }
            output_value = output_value
                .checked_add(output.note.amount().units())
                .ok_or(Error::Overflow)?;
            outputs.push(output.binding()?);
        }
        if input_value != output_value
            || nullifier_digest(&self.nullifiers(keys)?)? != claim.nullifiers
            || output_digest(&outputs)? != claim.outputs
        {
            return Err(Error::Invalid("funding value or output binding"));
        }
        Ok(())
    }

    pub fn authorize(
        self,
        consent: &FundingConsent<'_>,
        keys: &NoteKeys,
    ) -> Result<SpvPoolWitness> {
        self.check(consent, keys)?;
        let operation = self.claim.journal_digest()?;
        let inputs = self
            .inputs
            .into_iter()
            .map(|input| NoteInput::authorize(input.note, input.path, keys, operation))
            .collect::<Result<Vec<_>>>()?;
        let witness = SpvPoolWitness {
            policy: self.policy,
            claim: self.claim,
            inputs,
            outputs: self.change,
            action: SpvAction::Reserve(self.reservation),
        };
        witness.evaluate()?;
        Ok(witness)
    }
}
