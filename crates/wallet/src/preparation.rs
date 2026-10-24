use crate::{
    Result,
    prepare::{MAX_PCZT, inspect_payment_effects},
};
use pczt::{Pczt, roles::redactor::Redactor};
use zeroize::Zeroizing;
use zutti_protocol::{
    Digest, Secret32,
    encoding::{Canonical, Reader, Writer},
    escrow::ReservationOpening,
    order::OrderTerms,
};

pub const MAX_PREPARATION_WITNESS: usize = MAX_PCZT + 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparationClaim {
    domain: Digest,
    order: Digest,
    reservation: Digest,
}
impl PreparationClaim {
    pub const fn domain(&self) -> Digest {
        self.domain
    }
    pub const fn order(&self) -> Digest {
        self.order
    }
    pub const fn reservation(&self) -> Digest {
        self.reservation
    }
}
impl Canonical for PreparationClaim {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        writer.fixed(b"ZUTTIPRP");
        writer.u16(1);
        writer.fixed(&self.domain.0);
        writer.fixed(&self.order.0);
        writer.fixed(&self.reservation.0);
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTIPRP" || reader.u16()? != 1 {
            return Err(zutti_protocol::Error::Version);
        }
        Ok(Self {
            domain: Digest::nonzero(reader.fixed()?)?,
            order: Digest::nonzero(reader.fixed()?)?,
            reservation: Digest::nonzero(reader.fixed()?)?,
        })
    }
}

pub struct PreparationWitness {
    pub(crate) terms: OrderTerms,
    pub(crate) order_randomness: Secret32,
    pub(crate) reservation_randomness: Secret32,
    pub(crate) pczt: Zeroizing<Vec<u8>>,
}
impl core::fmt::Debug for PreparationWitness {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("PreparationWitness([REDACTED])")
    }
}
impl PreparationWitness {
    pub fn new(
        terms: OrderTerms,
        order_randomness: Secret32,
        reservation_randomness: Secret32,
        pczt: Vec<u8>,
    ) -> Result<Self> {
        if pczt.is_empty() || pczt.len() > MAX_PCZT {
            return Err(crate::Error::Pczt);
        }
        let pczt = Zeroizing::new(pczt);
        let pczt = Pczt::parse(&pczt).map_err(|_| crate::Error::Pczt)?;
        let pczt = Redactor::new(pczt)
            .redact_global_with(|mut global| global.clear_proprietary())
            .redact_ironwood_with(|mut bundle| {
                bundle.clear_bsk();
                bundle.clear_zkproof();
                bundle.redact_actions(|mut action| {
                    action.clear_spend_auth_sig();
                    action.clear_spend_recipient();
                    action.clear_spend_value();
                    action.clear_spend_rho();
                    action.clear_spend_rseed();
                    action.clear_spend_fvk();
                    action.clear_spend_witness();
                    action.clear_spend_alpha();
                    action.clear_spend_dummy_sk();
                    action.clear_rcv();
                });
            })
            .finish()
            .serialize()
            .map_err(|_| crate::Error::Pczt)?;
        Ok(Self {
            terms,
            order_randomness,
            reservation_randomness,
            pczt: Zeroizing::new(pczt),
        })
    }
    pub fn evaluate(&self) -> Result<PreparationClaim> {
        let payment = inspect_payment_effects(&self.pczt, &self.terms, &self.order_randomness)?;
        let domain = self.terms.domain().id()?;
        let order = self.terms.commitment(&self.order_randomness)?;
        Ok(PreparationClaim {
            domain,
            order,
            reservation: ReservationOpening::bind(
                domain,
                order,
                payment.txid(),
                &self.reservation_randomness,
            )?,
        })
    }
}
impl Canonical for PreparationWitness {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        writer.fixed(b"ZUTTIPWT");
        writer.u16(1);
        self.terms.encode(writer)?;
        writer.fixed(self.order_randomness.expose());
        writer.fixed(self.reservation_randomness.expose());
        writer.bytes(&self.pczt)
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTIPWT" || reader.u16()? != 1 {
            return Err(zutti_protocol::Error::Version);
        }
        Self::new(
            OrderTerms::decode(reader)?,
            Secret32::from_bytes(reader.fixed()?)?,
            Secret32::from_bytes(reader.fixed()?)?,
            reader.bytes(MAX_PCZT)?.to_vec(),
        )
        .map_err(|_| zutti_protocol::Error::Length)
    }
}
