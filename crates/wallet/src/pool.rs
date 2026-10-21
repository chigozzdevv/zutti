pub mod escrow;
use crate::private_note::{EncryptedNote, NoteEncryption, NoteKeys};
use escrow::EscrowNote;
use sha2::Digest as _;
use zutti_protocol::{
    Digest, Error, Result, Secret32,
    encoding::{Canonical, Reader, Writer},
    escrow::ReservationOpening,
    note::FixedOutput,
    order::OrderTerms,
    pool::{OperationKind, PoolClaim, PoolPath, nullifier_digest, output_digest},
    private_note::{PrivateNote, SpendAuthorization},
};
use zutti_source::{
    IndexProof,
    checkpoint::{HeaderPath, SourceCheckpoint},
    query::QueryWitness,
};

pub const MAX_POOL_WITNESS: usize = 131_072;

pub struct NoteInput {
    pub note: PrivateNote,
    pub path: PoolPath,
    pub nullifier_key: Secret32,
    pub authorization: SpendAuthorization,
}
impl core::fmt::Debug for NoteInput {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("NoteInput([REDACTED])")
    }
}
impl NoteInput {
    pub fn authorize(
        note: PrivateNote,
        path: PoolPath,
        keys: &NoteKeys,
        operation: Digest,
    ) -> Result<Self> {
        let authorization = keys.authorize(&note, operation)?;
        Ok(Self {
            note,
            path,
            nullifier_key: keys.nullifier_secret()?,
            authorization,
        })
    }
    fn verify(&self, claim: &PoolClaim) -> Result<Digest> {
        if self.note.domain() != claim.domain {
            return Err(Error::Invalid("input note domain"));
        }
        self.path
            .verify(claim.domain, self.note.commitment()?, claim.root)?;
        self.note
            .verify_spend(claim.journal_digest()?, &self.authorization)?;
        self.note.nullifier(&self.nullifier_key)
    }
}
impl Canonical for NoteInput {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        self.note.encode(writer)?;
        self.path.encode(writer)?;
        writer.fixed(self.nullifier_key.expose());
        self.authorization.encode(writer)
    }
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            note: PrivateNote::decode(reader)?,
            path: PoolPath::decode(reader)?,
            nullifier_key: Secret32::from_bytes(reader.fixed()?)?,
            authorization: SpendAuthorization::decode(reader)?,
        })
    }
}

pub struct NoteOutput {
    pub note: PrivateNote,
    pub encryption: NoteEncryption,
}
impl core::fmt::Debug for NoteOutput {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("NoteOutput([REDACTED])")
    }
}
impl NoteOutput {
    pub fn encrypted(&self) -> Result<EncryptedNote> {
        EncryptedNote::encrypt_with(&self.note, &self.encryption)
    }
    pub fn binding(&self) -> Result<(Digest, Digest)> {
        let encrypted = self.encrypted()?;
        Ok((self.note.commitment()?, encrypted.digest()?))
    }
    pub fn fixed(&self) -> Result<FixedOutput> {
        self.encrypted()?.fixed_output(&self.note)
    }
}
impl Canonical for NoteOutput {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        self.note.encode(writer)?;
        self.encryption.encode(writer)
    }
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            note: PrivateNote::decode(reader)?,
            encryption: NoteEncryption::decode(reader)?,
        })
    }
}

pub struct ReservationWitness {
    pub terms: OrderTerms,
    pub order_randomness: Secret32,
    pub reservation_randomness: Secret32,
    pub txid: Digest,
    pub escrow: EscrowNote,
    pub nonce: [u8; 24],
    pub checkpoint: SourceCheckpoint,
    pub anchor: HeaderPath,
    pub payout: NoteOutput,
    pub refund: NoteOutput,
}
impl core::fmt::Debug for ReservationWitness {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ReservationWitness([REDACTED])")
    }
}
impl ReservationWitness {
    pub fn binding(&self) -> Result<(Digest, Digest)> {
        Ok((
            self.escrow.commitment()?,
            Digest(sha2::Sha256::digest(self.escrow.encrypt(self.nonce)?).into()),
        ))
    }
    pub(crate) fn verify(&self, claim: &PoolClaim) -> Result<()> {
        check_source(claim, &self.terms, &self.checkpoint, &self.anchor)?;
        let order = self.terms.commitment(&self.order_randomness)?;
        let reservation =
            ReservationOpening::bind(claim.domain, order, self.txid, &self.reservation_randomness)?;
        if claim.order_tag != self.terms.use_tag()?
            || claim.order != order
            || claim.reservation != reservation
            || self.escrow.domain != claim.domain
            || self.escrow.order != order
            || self.escrow.reservation != reservation
            || self.escrow.amount != self.terms.usdc().units()
            || self.payout.note.domain() != claim.domain
            || self.refund.note.domain() != claim.domain
            || self.payout.fixed()? != self.terms.payout()
            || self.refund.fixed()? != self.terms.refund()
        {
            return Err(Error::Invalid("conditional reservation binding"));
        }
        Ok(())
    }
}
impl Canonical for ReservationWitness {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        self.terms.encode(writer)?;
        writer.fixed(self.order_randomness.expose());
        writer.fixed(self.reservation_randomness.expose());
        writer.fixed(&self.txid.0);
        self.escrow.encode(writer)?;
        writer.fixed(&self.nonce);
        self.checkpoint.encode(writer)?;
        self.anchor.encode(writer)?;
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
            checkpoint: SourceCheckpoint::decode(reader)?,
            anchor: HeaderPath::decode(reader)?,
            payout: NoteOutput::decode(reader)?,
            refund: NoteOutput::decode(reader)?,
        })
    }
}

pub struct SettlementWitness {
    pub terms: OrderTerms,
    pub order_randomness: Secret32,
    pub reservation_randomness: Secret32,
    pub txid: Digest,
    pub escrow: EscrowNote,
    pub path: PoolPath,
    pub checkpoint: SourceCheckpoint,
    pub anchor: HeaderPath,
    pub payment: IndexProof,
}
impl core::fmt::Debug for SettlementWitness {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SettlementWitness([REDACTED])")
    }
}
impl SettlementWitness {
    fn verify(&self, claim: &PoolClaim, output: &NoteOutput) -> Result<Digest> {
        check_source(claim, &self.terms, &self.checkpoint, &self.anchor)?;
        self.path
            .verify(claim.domain, self.escrow.commitment()?, claim.root)?;
        let query = QueryWitness::new(
            self.terms.clone(),
            Secret32::from_bytes(*self.order_randomness.expose())?,
            Secret32::from_bytes(*self.reservation_randomness.expose())?,
            self.txid,
            self.payment.clone(),
            self.checkpoint.stable.index,
        );
        let terminal = query
            .evaluate()
            .map_err(|_| Error::Invalid("source payment outcome"))?;
        let (commitment, ciphertext) = output.binding()?;
        if self.escrow.domain != claim.domain
            || self.escrow.order != terminal.order()
            || self.escrow.reservation != terminal.reservation()
            || self.escrow.amount != self.terms.usdc().units()
            || output.note.amount().units() != self.escrow.amount
            || output.note.domain() != claim.domain
            || terminal.output() != commitment
            || terminal.ciphertext() != ciphertext
        {
            return Err(Error::Invalid("conditional settlement output"));
        }
        self.escrow.nullifier()
    }
}
impl Canonical for SettlementWitness {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        self.terms.encode(writer)?;
        writer.fixed(self.order_randomness.expose());
        writer.fixed(self.reservation_randomness.expose());
        writer.fixed(&self.txid.0);
        self.escrow.encode(writer)?;
        self.path.encode(writer)?;
        self.checkpoint.encode(writer)?;
        self.anchor.encode(writer)?;
        self.payment.encode(writer)
    }
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            terms: OrderTerms::decode(reader)?,
            order_randomness: Secret32::from_bytes(reader.fixed()?)?,
            reservation_randomness: Secret32::from_bytes(reader.fixed()?)?,
            txid: Digest::nonzero(reader.fixed()?)?,
            escrow: EscrowNote::decode(reader)?,
            path: PoolPath::decode(reader)?,
            checkpoint: SourceCheckpoint::decode(reader)?,
            anchor: HeaderPath::decode(reader)?,
            payment: IndexProof::decode(reader)?,
        })
    }
}
fn check_source(
    claim: &PoolClaim,
    terms: &OrderTerms,
    checkpoint: &SourceCheckpoint,
    anchor: &HeaderPath,
) -> Result<()> {
    let context = checkpoint.stable.index.context;
    if terms.domain().id()? != claim.domain
        || context.genesis() != terms.domain().source_genesis()
        || context.rules() != terms.domain().source_policy()
        || checkpoint.id()? != claim.checkpoint
        || context.start() != terms.window().index_start()
    {
        return Err(Error::Invalid("source checkpoint domain"));
    }
    checkpoint
        .verify_anchor(
            terms.window().anchor_height(),
            terms.window().anchor_hash(),
            anchor,
        )
        .map_err(|_| Error::Invalid("source order ancestry"))
}

#[derive(Debug)]
pub enum PoolAction {
    Deposit,
    Transfer,
    Withdraw,
    Reserve(Box<ReservationWitness>),
    Settle(Box<SettlementWitness>),
}
impl PoolAction {
    fn kind(&self) -> OperationKind {
        match self {
            Self::Deposit => OperationKind::Deposit,
            Self::Transfer => OperationKind::Transfer,
            Self::Withdraw => OperationKind::Withdraw,
            Self::Reserve(_) => OperationKind::Reserve,
            Self::Settle(_) => OperationKind::Settle,
        }
    }
}
pub struct PoolWitness {
    pub claim: PoolClaim,
    pub inputs: Vec<NoteInput>,
    pub outputs: Vec<NoteOutput>,
    pub action: PoolAction,
}
impl core::fmt::Debug for PoolWitness {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("PoolWitness([REDACTED])")
    }
}
impl PoolWitness {
    pub fn evaluate(&self) -> Result<PoolClaim> {
        self.claim.validate()?;
        if self.claim.kind != self.action.kind() || self.inputs.len() > 8 || self.outputs.len() > 8
        {
            return Err(Error::Invalid("pool operation shape"));
        }
        let mut nullifiers = Vec::new();
        let mut outputs = Vec::new();
        let mut input_value = 0_u128;
        let mut output_value = 0_u128;
        for input in &self.inputs {
            nullifiers.push(input.verify(&self.claim)?);
            input_value = input_value
                .checked_add(input.note.amount().units())
                .ok_or(Error::Overflow)?;
        }
        for output in &self.outputs {
            if output.note.domain() != self.claim.domain {
                return Err(Error::Invalid("output note domain"));
            }
            outputs.push(output.binding()?);
            output_value = output_value
                .checked_add(output.note.amount().units())
                .ok_or(Error::Overflow)?;
        }
        match &self.action {
            PoolAction::Deposit => {
                if !self.inputs.is_empty() || self.outputs.len() != 1 {
                    return Err(Error::Invalid("deposit shape"));
                }
                input_value = u128::from(self.claim.value);
            }
            PoolAction::Transfer => {
                if self.inputs.is_empty() || self.outputs.is_empty() {
                    return Err(Error::Invalid("transfer shape"));
                }
            }
            PoolAction::Withdraw => {
                if self.inputs.is_empty() {
                    return Err(Error::Invalid("withdrawal shape"));
                }
                output_value = output_value
                    .checked_add(u128::from(self.claim.value))
                    .ok_or(Error::Overflow)?;
            }
            PoolAction::Reserve(reservation) => {
                if self.inputs.is_empty() || self.outputs.len() > 7 {
                    return Err(Error::Invalid("reservation shape"));
                }
                reservation.verify(&self.claim)?;
                outputs.insert(0, reservation.binding()?);
                output_value = output_value
                    .checked_add(reservation.escrow.amount)
                    .ok_or(Error::Overflow)?;
            }
            PoolAction::Settle(settlement) => {
                if !self.inputs.is_empty() || self.outputs.len() != 1 {
                    return Err(Error::Invalid("settlement shape"));
                }
                nullifiers.push(settlement.verify(&self.claim, &self.outputs[0])?);
                input_value = settlement.escrow.amount;
            }
        }
        if input_value != output_value
            || nullifier_digest(&nullifiers)? != self.claim.nullifiers
            || output_digest(&outputs)? != self.claim.outputs
        {
            return Err(Error::Invalid("pool conservation or public binding"));
        }
        Ok(self.claim.clone())
    }
}
impl Canonical for PoolWitness {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        if self.inputs.len() > 8 || self.outputs.len() > 8 {
            return Err(Error::Length);
        }
        writer.fixed(b"ZUTTIPWT");
        writer.u16(1);
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
            PoolAction::Reserve(reservation) => reservation.encode(writer)?,
            PoolAction::Settle(settlement) => settlement.encode(writer)?,
            _ => {}
        }
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTIPWT" || reader.u16()? != 1 {
            return Err(Error::Version);
        }
        let claim = PoolClaim::decode(reader)?;
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
            OperationKind::Deposit => PoolAction::Deposit,
            OperationKind::Transfer => PoolAction::Transfer,
            OperationKind::Withdraw => PoolAction::Withdraw,
            OperationKind::Reserve => {
                PoolAction::Reserve(Box::new(ReservationWitness::decode(reader)?))
            }
            OperationKind::Settle => {
                PoolAction::Settle(Box::new(SettlementWitness::decode(reader)?))
            }
        };
        Ok(Self {
            claim,
            inputs,
            outputs,
            action,
        })
    }
}
