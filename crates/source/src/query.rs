use crate::{
    Error, IndexProof, IndexSnapshot, Result,
    history::{CandidateOutcome, classify_candidate},
};
use zutti_protocol::{
    Digest, Secret32,
    encoding::{Canonical, Reader, Writer},
    escrow::ReservationOpening,
    order::OrderTerms,
};

pub const MAX_QUERY_WITNESS: usize = 32_768;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryClaim {
    domain: Digest,
    order: Digest,
    reservation: Digest,
    snapshot: IndexSnapshot,
    output: Digest,
    ciphertext: Digest,
}
impl QueryClaim {
    pub const fn domain(&self) -> Digest {
        self.domain
    }
    pub const fn order(&self) -> Digest {
        self.order
    }
    pub const fn reservation(&self) -> Digest {
        self.reservation
    }
    pub const fn snapshot(&self) -> IndexSnapshot {
        self.snapshot
    }
    pub const fn output(&self) -> Digest {
        self.output
    }
    pub const fn ciphertext(&self) -> Digest {
        self.ciphertext
    }
}
impl Canonical for QueryClaim {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        writer.fixed(b"ZUTTIQRY");
        writer.u16(1);
        writer.fixed(&self.domain.0);
        writer.fixed(&self.order.0);
        writer.fixed(&self.reservation.0);
        self.snapshot.encode(writer)?;
        writer.fixed(&self.output.0);
        writer.fixed(&self.ciphertext.0);
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTIQRY" || reader.u16()? != 1 {
            return Err(zutti_protocol::Error::Version);
        }
        Ok(Self {
            domain: Digest::nonzero(reader.fixed()?)?,
            order: Digest::nonzero(reader.fixed()?)?,
            reservation: Digest::nonzero(reader.fixed()?)?,
            snapshot: IndexSnapshot::decode(reader)?,
            output: Digest::nonzero(reader.fixed()?)?,
            ciphertext: Digest::nonzero(reader.fixed()?)?,
        })
    }
}

pub struct QueryWitness {
    terms: OrderTerms,
    order_randomness: Secret32,
    reservation_randomness: Secret32,
    txid: Digest,
    path: IndexProof,
    snapshot: IndexSnapshot,
}
impl core::fmt::Debug for QueryWitness {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("QueryWitness([REDACTED])")
    }
}
impl QueryWitness {
    pub fn new(
        terms: OrderTerms,
        order_randomness: Secret32,
        reservation_randomness: Secret32,
        txid: Digest,
        path: IndexProof,
        snapshot: IndexSnapshot,
    ) -> Self {
        Self {
            terms,
            order_randomness,
            reservation_randomness,
            txid,
            path,
            snapshot,
        }
    }
    pub fn evaluate(&self) -> Result<QueryClaim> {
        let domain = self.terms.domain();
        if self.snapshot.context.genesis() != domain.source_genesis()
            || self.snapshot.context.rules() != domain.source_policy()
        {
            return Err(Error::Window);
        }
        let output =
            match classify_candidate(self.terms.window(), self.snapshot, &self.path, self.txid)? {
                CandidateOutcome::Paid { .. } => self.terms.payout(),
                CandidateOutcome::ExpiredAbsent => self.terms.refund(),
                CandidateOutcome::Waiting => return Err(Error::Window),
            };
        let order = self.terms.commitment(&self.order_randomness)?;
        Ok(QueryClaim {
            domain: domain.id()?,
            order,
            reservation: ReservationOpening::bind(
                domain.id()?,
                order,
                self.txid,
                &self.reservation_randomness,
            )?,
            snapshot: self.snapshot,
            output: output.commitment(),
            ciphertext: output.ciphertext_digest(),
        })
    }
}
impl Canonical for QueryWitness {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        writer.fixed(b"ZUTTIQWT");
        writer.u16(1);
        self.terms.encode(writer)?;
        writer.fixed(self.order_randomness.expose());
        writer.fixed(self.reservation_randomness.expose());
        writer.fixed(&self.txid.0);
        self.path.encode(writer)?;
        self.snapshot.encode(writer)
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTIQWT" || reader.u16()? != 1 {
            return Err(zutti_protocol::Error::Version);
        }
        Ok(Self::new(
            OrderTerms::decode(reader)?,
            Secret32::from_bytes(reader.fixed()?)?,
            Secret32::from_bytes(reader.fixed()?)?,
            Digest::nonzero(reader.fixed()?)?,
            IndexProof::decode(reader)?,
            IndexSnapshot::decode(reader)?,
        ))
    }
}
