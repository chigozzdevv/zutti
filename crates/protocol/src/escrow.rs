use crate::{
    Digest, Result, Secret32,
    commitment::{commit, hash},
    order::OrderTerms,
};

pub struct ReservationOpening {
    order: Digest,
    txid: Digest,
    randomness: Secret32,
}
impl ReservationOpening {
    pub fn new(order: Digest, txid: Digest, randomness: Secret32) -> Result<Self> {
        Digest::nonzero(order.0)?;
        Digest::nonzero(txid.0)?;
        Ok(Self {
            order,
            txid,
            randomness,
        })
    }
    pub fn commitment(&self, terms: &OrderTerms, order_randomness: &Secret32) -> Result<Digest> {
        if terms.commitment(order_randomness)? != self.order {
            return Err(crate::Error::Invalid("order opening"));
        }
        Self::bind(
            terms.domain().id()?,
            self.order,
            self.txid,
            &self.randomness,
        )
    }
    pub fn bind(
        domain: Digest,
        order: Digest,
        txid: Digest,
        randomness: &Secret32,
    ) -> Result<Digest> {
        Digest::nonzero(domain.0)?;
        Digest::nonzero(order.0)?;
        Digest::nonzero(txid.0)?;
        let mut payload = [0; 64];
        payload[..32].copy_from_slice(&order.0);
        payload[32..].copy_from_slice(&txid.0);
        Ok(commit(b"reservation", domain, &payload, randomness))
    }
    pub fn terminal_nullifier(domain: Digest, escrow_secret: &Secret32) -> Digest {
        hash(b"escrow-spend", &[&domain.0, escrow_secret.expose()])
    }
}
impl core::fmt::Debug for ReservationOpening {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ReservationOpening([REDACTED])")
    }
}
