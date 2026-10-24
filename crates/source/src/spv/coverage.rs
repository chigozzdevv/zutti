use super::{
    Evaluation, HeaderClaim, MAX_HEADERS, MAX_WITNESS, Result, WindowClaim, WindowTerms,
    parse_block,
};
use crate::block::MAX_BLOCK_BYTES;
use zutti_protocol::{
    Digest, Secret32,
    encoding::{Canonical, Reader, Writer},
};

#[derive(Clone)]
pub struct CoverageClaim {
    pub terms: Digest,
    pub reservation: Digest,
    pub anchor: Digest,
    pub first: u32,
    pub expiry: u32,
    pub payment: u32,
    pub head: HeaderClaim,
}
impl core::fmt::Debug for CoverageClaim {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("CoverageClaim([REDACTED])")
    }
}
impl Canonical for CoverageClaim {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        for digest in [self.terms, self.reservation, self.anchor] {
            Digest::nonzero(digest.0)?;
        }
        let height = self
            .head
            .height()
            .map_err(|_| zutti_protocol::Error::Invalid("SPV coverage height"))?;
        if self.first == 0
            || self.expiry < self.first
            || height < self.first
            || height > self.expiry
            || self.expiry - self.first + 1 > self.head.policy().window_blocks()
            || (self.payment != 0 && (self.payment < self.first || self.payment > height))
            || u64::from(self.expiry) + u64::from(self.head.policy().confirmations())
                > u64::from(self.head.policy().valid_through())
        {
            return Err(zutti_protocol::Error::Invalid("SPV coverage claim"));
        }
        writer.fixed(b"ZUTTISPG");
        writer.u16(1);
        writer.fixed(&self.terms.0);
        writer.fixed(&self.reservation.0);
        writer.fixed(&self.anchor.0);
        writer.u32(self.first);
        writer.u32(self.expiry);
        writer.u32(self.payment);
        self.head.encode(writer)
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTISPG" || reader.u16()? != 1 {
            return Err(zutti_protocol::Error::Version);
        }
        let claim = Self {
            terms: Digest(reader.fixed()?),
            reservation: Digest(reader.fixed()?),
            anchor: Digest(reader.fixed()?),
            first: reader.u32()?,
            expiry: reader.u32()?,
            payment: reader.u32()?,
            head: HeaderClaim::decode(reader)?,
        };
        claim.to_bytes()?;
        Ok(claim)
    }
}
#[derive(Debug, Clone)]
pub enum CoveragePrevious {
    Prefix(HeaderClaim),
    Coverage(CoverageClaim),
}
impl CoveragePrevious {
    pub fn head(&self) -> &HeaderClaim {
        match self {
            Self::Prefix(head) => head,
            Self::Coverage(claim) => &claim.head,
        }
    }
    pub fn journal(&self) -> zutti_protocol::Result<Vec<u8>> {
        match self {
            Self::Prefix(head) => head.to_bytes(),
            Self::Coverage(claim) => claim.to_bytes(),
        }
    }
}
pub struct CoverageWitness {
    pub terms: WindowTerms,
    pub txid: Digest,
    pub randomness: Secret32,
    pub previous: CoveragePrevious,
    pub blocks: Vec<Vec<u8>>,
    pub confirmations: Vec<Vec<u8>>,
    pub finish: bool,
}
impl core::fmt::Debug for CoverageWitness {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("CoverageWitness([REDACTED])")
    }
}
impl CoverageWitness {
    pub fn evaluate(&self) -> Result<Evaluation> {
        use super::Error;
        self.check_bounds()?;
        let policy = self.previous.head().policy();
        let first = self
            .terms
            .anchor_height()
            .checked_add(1)
            .ok_or(Error::Bounds)?;
        let reservation = self
            .terms
            .reservation(policy, self.txid, &self.randomness)?;
        if self.terms.expiry() - self.terms.anchor_height() > policy.window_blocks()
            || u64::from(self.terms.expiry()) + u64::from(policy.confirmations())
                > u64::from(policy.valid_through())
        {
            return Err(Error::Policy);
        }
        let (anchor, mut payment) = match &self.previous {
            CoveragePrevious::Prefix(previous) => {
                if previous.height()? != self.terms.anchor_height()
                    || previous.tip()? != self.terms.anchor_hash()
                {
                    return Err(Error::Coverage);
                }
                (previous.id()?, 0)
            }
            CoveragePrevious::Coverage(previous) => {
                previous.to_bytes()?;
                if previous.terms != self.terms.id()?
                    || previous.reservation != reservation
                    || previous.first != first
                    || previous.expiry != self.terms.expiry()
                {
                    return Err(Error::Coverage);
                }
                (previous.anchor, previous.payment)
            }
        };
        let mut head = self.previous.head().clone();
        for bytes in &self.blocks {
            let block = parse_block(bytes, policy.network())?;
            let height = head.height()?.checked_add(1).ok_or(Error::Bounds)?;
            if height > self.terms.expiry() || block.claimed_height() != height {
                return Err(Error::Coverage);
            }
            let mut header = Vec::new();
            block
                .header()
                .write(&mut header)
                .map_err(|_| crate::Error::Block)?;
            head = head.advance(&[header])?;
            if payment == 0 && block.txids().contains(&self.txid) {
                payment = height;
            }
        }
        let coverage = CoverageClaim {
            terms: self.terms.id()?,
            reservation,
            anchor,
            first,
            expiry: self.terms.expiry(),
            payment,
            head,
        };
        coverage.to_bytes()?;
        if !self.finish {
            return Ok(Evaluation::Coverage(coverage));
        }
        if coverage.head.height()? != self.terms.expiry()
            || self.confirmations.len() != policy.confirmations() as usize
        {
            return Err(Error::Coverage);
        }
        let end = coverage.head.advance(&self.confirmations)?;
        Ok(Evaluation::Window(WindowClaim {
            policy: policy.id()?,
            prefix: anchor,
            checkpoint: end.id()?,
            reservation,
            height: end.height()?,
            tip: end.tip()?,
            work: end.work()?,
            output: if payment == 0 {
                self.terms.refund()
            } else {
                self.terms.payout()
            },
        }))
    }
    fn check_bounds(&self) -> Result<()> {
        self.terms.to_bytes()?;
        Digest::nonzero(self.txid.0)?;
        let bytes: usize = self
            .blocks
            .iter()
            .chain(&self.confirmations)
            .map(Vec::len)
            .sum();
        if self.blocks.len() > super::MAX_WINDOW_BLOCKS as usize
            || self.confirmations.len() > MAX_HEADERS
            || self
                .blocks
                .iter()
                .any(|b| b.is_empty() || b.len() > MAX_BLOCK_BYTES)
            || self
                .confirmations
                .iter()
                .any(|h| h.len() != super::HEADER_BYTES)
            || bytes > MAX_WITNESS - 16_384
            || (!self.finish && (self.blocks.is_empty() || !self.confirmations.is_empty()))
        {
            return Err(super::Error::Bounds);
        }
        Ok(())
    }
}
impl Canonical for CoverageWitness {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        self.check_bounds()
            .map_err(|_| zutti_protocol::Error::Length)?;
        self.terms.encode(writer)?;
        writer.fixed(&self.txid.0);
        writer.fixed(self.randomness.expose());
        match &self.previous {
            CoveragePrevious::Prefix(previous) => {
                writer.u8(1);
                previous.encode(writer)?;
            }
            CoveragePrevious::Coverage(previous) => {
                writer.u8(2);
                previous.encode(writer)?;
            }
        }
        writer.u8(u8::from(self.finish));
        writer.u8(self.blocks.len() as u8);
        for block in &self.blocks {
            writer.bytes(block)?;
        }
        writer.u32(self.confirmations.len() as u32);
        for header in &self.confirmations {
            writer.fixed(header);
        }
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        let terms = WindowTerms::decode(reader)?;
        let txid = Digest::nonzero(reader.fixed()?)?;
        let randomness = Secret32::from_bytes(reader.fixed()?)?;
        let previous = match reader.u8()? {
            1 => CoveragePrevious::Prefix(HeaderClaim::decode(reader)?),
            2 => CoveragePrevious::Coverage(CoverageClaim::decode(reader)?),
            _ => return Err(zutti_protocol::Error::Version),
        };
        let finish = match reader.u8()? {
            0 => false,
            1 => true,
            _ => return Err(zutti_protocol::Error::Version),
        };
        let count = reader.u8()? as usize;
        if count > super::MAX_WINDOW_BLOCKS as usize {
            return Err(zutti_protocol::Error::Length);
        }
        let mut blocks = Vec::new();
        for _ in 0..count {
            blocks.push(reader.bytes(MAX_BLOCK_BYTES)?.to_vec());
        }
        let count = reader.u32()? as usize;
        if count > MAX_HEADERS {
            return Err(zutti_protocol::Error::Length);
        }
        let mut confirmations = Vec::new();
        for _ in 0..count {
            confirmations.push(reader.fixed::<{ super::HEADER_BYTES }>()?.to_vec());
        }
        let witness = Self {
            terms,
            txid,
            randomness,
            previous,
            blocks,
            confirmations,
            finish,
        };
        witness
            .check_bounds()
            .map_err(|_| zutti_protocol::Error::Length)?;
        Ok(witness)
    }
}
