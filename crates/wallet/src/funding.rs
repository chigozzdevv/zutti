use crate::{
    pool::{NoteInput, NoteOutput},
    private_note::NoteKeys,
    spv_pool::{SpvAction, SpvPoolClaim, SpvPoolWitness, SpvReservation},
};
use zutti_protocol::{
    Digest, Error, Result,
    order::OrderTerms,
    pool::{OperationKind, PoolPath, nullifier_digest, output_digest},
    private_note::{NoteRecipient, PrivateNote},
};
use zutti_source::spv::Policy;

pub struct FundingInput {
    pub note: PrivateNote,
    pub path: PoolPath,
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
