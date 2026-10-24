use crate::{
    Error, Result,
    preparation::{MAX_PREPARATION_WITNESS, PreparationWitness},
    prepare::inspect_payment_effects,
};
use zutti_protocol::{
    Digest,
    commitment::hash,
    encoding::{Canonical, Reader, Writer},
    escrow::ReservationOpening,
};
use zutti_source::spv::{Policy, WindowClaim, WindowTerms};

pub const MAX_BINDING_WITNESS: usize = MAX_PREPARATION_WITNESS + 128;

pub fn payment_profile() -> Digest {
    hash(b"payment-profile", &[b"ironwood-v6-anchor-dependency-v1"])
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaymentBindingClaim {
    domain: Digest,
    order: Digest,
    reservation: Digest,
    policy: Digest,
    window: Digest,
    source_reservation: Digest,
    profile: Digest,
}
impl PaymentBindingClaim {
    pub const fn domain(&self) -> Digest {
        self.domain
    }
    pub const fn order(&self) -> Digest {
        self.order
    }
    pub const fn reservation(&self) -> Digest {
        self.reservation
    }
    pub const fn policy(&self) -> Digest {
        self.policy
    }
    pub const fn window(&self) -> Digest {
        self.window
    }
    pub const fn source_reservation(&self) -> Digest {
        self.source_reservation
    }
    pub fn require_window(&self, terms: &WindowTerms, claim: &WindowClaim) -> Result<()> {
        if terms.id()? != self.window
            || claim.policy != self.policy
            || claim.reservation != self.source_reservation
            || (claim.output != terms.payout() && claim.output != terms.refund())
        {
            return Err(Error::Payment);
        }
        claim.to_bytes()?;
        Ok(())
    }
}
impl Canonical for PaymentBindingClaim {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        if self.profile != payment_profile() {
            return Err(zutti_protocol::Error::Invalid("payment binding profile"));
        }
        writer.fixed(b"ZUTTIBND");
        writer.u16(1);
        for digest in [
            self.domain,
            self.order,
            self.reservation,
            self.policy,
            self.window,
            self.source_reservation,
            self.profile,
        ] {
            writer.fixed(&Digest::nonzero(digest.0)?.0);
        }
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTIBND" || reader.u16()? != 1 {
            return Err(zutti_protocol::Error::Version);
        }
        let claim = Self {
            domain: Digest(reader.fixed()?),
            order: Digest(reader.fixed()?),
            reservation: Digest(reader.fixed()?),
            policy: Digest(reader.fixed()?),
            window: Digest(reader.fixed()?),
            source_reservation: Digest(reader.fixed()?),
            profile: Digest(reader.fixed()?),
        };
        claim.to_bytes()?;
        Ok(claim)
    }
}

pub struct PaymentBindingWitness {
    policy: Policy,
    preparation: PreparationWitness,
}
impl core::fmt::Debug for PaymentBindingWitness {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("PaymentBindingWitness([REDACTED])")
    }
}
impl PaymentBindingWitness {
    pub fn new(policy: Policy, preparation: PreparationWitness) -> Result<Self> {
        policy.to_bytes()?;
        Ok(Self {
            policy,
            preparation,
        })
    }
    pub fn evaluate(&self) -> Result<PaymentBindingClaim> {
        let terms = &self.preparation.terms;
        let window = terms.window();
        let policy = self.policy.id()?;
        if terms.domain().payment_profile() != payment_profile()
            || terms.domain().source_genesis()
                != zutti_source::bootstrap::genesis_hash(self.policy.network())
                    .map_err(|_| Error::Payment)?
            || terms.domain().source_policy() != policy
            || window.expiry() - window.anchor_height() > self.policy.window_blocks()
            || u64::from(window.expiry()) + u64::from(self.policy.confirmations())
                > u64::from(self.policy.valid_through())
        {
            return Err(Error::Payment);
        }
        let payment = inspect_payment_effects(
            &self.preparation.pczt,
            terms,
            &self.preparation.order_randomness,
        )?;
        let domain = terms.domain().id()?;
        let order = terms.commitment(&self.preparation.order_randomness)?;
        let source = WindowTerms::new(
            domain,
            order,
            window.anchor_height(),
            window.anchor_hash(),
            window.expiry(),
            terms.payout().commitment(),
            terms.refund().commitment(),
        )?;
        Ok(PaymentBindingClaim {
            domain,
            order,
            reservation: ReservationOpening::bind(
                domain,
                order,
                payment.txid(),
                &self.preparation.reservation_randomness,
            )?,
            policy,
            window: source.id()?,
            source_reservation: source
                .reservation(
                    self.policy,
                    payment.txid(),
                    &self.preparation.reservation_randomness,
                )
                .map_err(|_| Error::Payment)?,
            profile: payment_profile(),
        })
    }
}
impl Canonical for PaymentBindingWitness {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        writer.fixed(b"ZUTTIBWI");
        writer.u16(1);
        self.policy.encode(writer)?;
        self.preparation.encode(writer)
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTIBWI" || reader.u16()? != 1 {
            return Err(zutti_protocol::Error::Version);
        }
        Self::new(Policy::decode(reader)?, PreparationWitness::decode(reader)?)
            .map_err(|_| zutti_protocol::Error::Invalid("payment binding witness"))
    }
    fn from_bytes(bytes: &[u8]) -> zutti_protocol::Result<Self> {
        if bytes.is_empty() || bytes.len() > MAX_BINDING_WITNESS {
            return Err(zutti_protocol::Error::Length);
        }
        let mut reader = Reader::new(bytes);
        let witness = Self::decode(&mut reader)?;
        reader.finish()?;
        Ok(witness)
    }
}
