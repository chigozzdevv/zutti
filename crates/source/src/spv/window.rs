use super::{Error, HeaderClaim, MAX_HEADERS, Result, parse_block, parse_header};
use crate::block::MAX_BLOCK_BYTES;
use zutti_protocol::{
    Digest, Secret32,
    commitment::commit,
    encoding::{Canonical, Reader, Writer},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Branch {
    Included,
    Absent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowTerms {
    domain: Digest,
    order: Digest,
    anchor_height: u32,
    anchor_hash: Digest,
    expiry: u32,
    payout: Digest,
    refund: Digest,
}

impl WindowTerms {
    pub fn new(
        domain: Digest,
        order: Digest,
        anchor_height: u32,
        anchor_hash: Digest,
        expiry: u32,
        payout: Digest,
        refund: Digest,
    ) -> zutti_protocol::Result<Self> {
        let terms = Self {
            domain,
            order,
            anchor_height,
            anchor_hash,
            expiry,
            payout,
            refund,
        };
        terms.to_bytes()?;
        Ok(terms)
    }
    pub const fn anchor_height(&self) -> u32 {
        self.anchor_height
    }
    pub const fn expiry(&self) -> u32 {
        self.expiry
    }
    pub const fn payout(&self) -> Digest {
        self.payout
    }
    pub const fn refund(&self) -> Digest {
        self.refund
    }
}

impl Canonical for WindowTerms {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        for digest in [
            self.domain,
            self.order,
            self.anchor_hash,
            self.payout,
            self.refund,
        ] {
            Digest::nonzero(digest.0)?;
        }
        if self.expiry <= self.anchor_height
            || self.expiry >= 500_000_000
            || self.payout == self.refund
        {
            return Err(zutti_protocol::Error::Invalid("SPV window terms"));
        }
        writer.fixed(b"ZUTTISPT");
        writer.u16(1);
        writer.fixed(&self.domain.0);
        writer.fixed(&self.order.0);
        writer.u32(self.anchor_height);
        writer.fixed(&self.anchor_hash.0);
        writer.u32(self.expiry);
        writer.fixed(&self.payout.0);
        writer.fixed(&self.refund.0);
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTISPT" || reader.u16()? != 1 {
            return Err(zutti_protocol::Error::Version);
        }
        Self::new(
            Digest(reader.fixed()?),
            Digest(reader.fixed()?),
            reader.u32()?,
            Digest(reader.fixed()?),
            reader.u32()?,
            Digest(reader.fixed()?),
            Digest(reader.fixed()?),
        )
    }
}

pub struct WindowWitness {
    terms: WindowTerms,
    txid: Digest,
    randomness: Secret32,
    branch: Branch,
    headers: Vec<Vec<u8>>,
    blocks: Vec<Vec<u8>>,
}

impl core::fmt::Debug for WindowWitness {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("WindowWitness([REDACTED])")
    }
}

impl WindowWitness {
    pub fn new(
        terms: WindowTerms,
        txid: Digest,
        randomness: Secret32,
        branch: Branch,
        headers: Vec<Vec<u8>>,
        blocks: Vec<Vec<u8>>,
    ) -> Result<Self> {
        let witness = Self {
            terms,
            txid,
            randomness,
            branch,
            headers,
            blocks,
        };
        witness.check_bounds()?;
        Ok(witness)
    }
    fn check_bounds(&self) -> Result<()> {
        self.terms.to_bytes()?;
        Digest::nonzero(self.txid.0)?;
        let bytes: usize = self.headers.iter().chain(&self.blocks).map(Vec::len).sum();
        if self.headers.is_empty()
            || self.headers.len() > MAX_HEADERS
            || self.headers.iter().any(|h| h.len() != super::HEADER_BYTES)
            || self.blocks.is_empty()
            || self.blocks.len() > super::MAX_WINDOW_BLOCKS as usize
            || self
                .blocks
                .iter()
                .any(|b| b.is_empty() || b.len() > MAX_BLOCK_BYTES)
            || bytes > super::MAX_WITNESS - 16_384
        {
            return Err(Error::Bounds);
        }
        Ok(())
    }
    pub fn reservation(&self, policy: super::Policy) -> Result<Digest> {
        let mut bytes = policy.id()?.0.to_vec();
        bytes.extend_from_slice(&self.terms.to_bytes()?);
        bytes.extend_from_slice(&self.txid.0);
        Ok(commit(
            b"spv-reservation",
            self.terms.domain,
            &bytes,
            &self.randomness,
        ))
    }
    pub fn evaluate(&self, previous: &HeaderClaim) -> Result<WindowClaim> {
        self.check_bounds()?;
        let policy = previous.policy();
        if self.terms.anchor_height != previous.height()?
            || self.terms.anchor_hash != previous.tip()?
            || self.terms.expiry - self.terms.anchor_height > policy.window_blocks()
            || u64::from(self.terms.expiry) + u64::from(policy.confirmations())
                > u64::from(policy.valid_through())
        {
            return Err(Error::Coverage);
        }
        let end = previous.advance(&self.headers)?;
        let mut inclusion = None;
        match self.branch {
            Branch::Included if self.blocks.len() != 1 => return Err(Error::Coverage),
            Branch::Absent
                if self.blocks.len() != (self.terms.expiry - self.terms.anchor_height) as usize =>
            {
                return Err(Error::Coverage);
            }
            _ => {}
        }
        for (offset, bytes) in self.blocks.iter().enumerate() {
            let block = parse_block(bytes, policy.network())?;
            let height = block.claimed_height();
            if height <= self.terms.anchor_height || height > self.terms.expiry {
                return Err(Error::Coverage);
            }
            if self.branch == Branch::Absent
                && height != self.terms.anchor_height + offset as u32 + 1
            {
                return Err(Error::Coverage);
            }
            let header_offset = (height - self.terms.anchor_height - 1) as usize;
            let header = self.headers.get(header_offset).ok_or(Error::Coverage)?;
            if Digest(parse_header(header)?.hash().0) != block.hash() {
                return Err(Error::Coverage);
            }
            if block.txids().contains(&self.txid) {
                inclusion = Some(height);
            }
        }
        let (required, output) = match (self.branch, inclusion) {
            (Branch::Included, Some(height)) => (
                height
                    .checked_add(policy.confirmations())
                    .ok_or(Error::Bounds)?,
                self.terms.payout,
            ),
            (Branch::Absent, None) => (
                self.terms
                    .expiry
                    .checked_add(policy.confirmations())
                    .ok_or(Error::Bounds)?,
                self.terms.refund,
            ),
            _ => return Err(Error::Outcome),
        };
        if end.height()? < required {
            return Err(Error::Confirmations);
        }
        Ok(WindowClaim {
            policy: policy.id()?,
            prefix: previous.id()?,
            reservation: self.reservation(policy)?,
            height: end.height()?,
            tip: end.tip()?,
            work: end.work()?,
            output,
        })
    }
}

impl Canonical for WindowWitness {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        self.check_bounds()
            .map_err(|_| zutti_protocol::Error::Invalid("SPV window bounds"))?;
        self.terms.encode(writer)?;
        writer.fixed(&self.txid.0);
        writer.fixed(self.randomness.expose());
        writer.u8(match self.branch {
            Branch::Included => 1,
            Branch::Absent => 2,
        });
        writer.u32(self.headers.len() as u32);
        for header in &self.headers {
            writer.fixed(header);
        }
        writer.u32(self.blocks.len() as u32);
        for block in &self.blocks {
            writer.bytes(block)?;
        }
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        let terms = WindowTerms::decode(reader)?;
        let txid = Digest::nonzero(reader.fixed()?)?;
        let randomness = Secret32::from_bytes(reader.fixed()?)?;
        let branch = match reader.u8()? {
            1 => Branch::Included,
            2 => Branch::Absent,
            _ => return Err(zutti_protocol::Error::Invalid("SPV window branch")),
        };
        let count = reader.u32()? as usize;
        if count == 0 || count > MAX_HEADERS {
            return Err(zutti_protocol::Error::Length);
        }
        let mut headers = Vec::with_capacity(count);
        for _ in 0..count {
            headers.push(reader.fixed::<{ super::HEADER_BYTES }>()?.to_vec());
        }
        let count = reader.u32()? as usize;
        if count == 0 || count > super::MAX_WINDOW_BLOCKS as usize {
            return Err(zutti_protocol::Error::Length);
        }
        let mut blocks = Vec::with_capacity(count);
        for _ in 0..count {
            blocks.push(reader.bytes(MAX_BLOCK_BYTES)?.to_vec());
        }
        Self::new(terms, txid, randomness, branch, headers, blocks)
            .map_err(|_| zutti_protocol::Error::Invalid("SPV window witness"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowClaim {
    pub policy: Digest,
    pub prefix: Digest,
    pub reservation: Digest,
    pub height: u32,
    pub tip: Digest,
    pub work: [u8; 32],
    pub output: Digest,
}

impl Canonical for WindowClaim {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        for digest in [
            self.policy,
            self.prefix,
            self.reservation,
            self.tip,
            self.output,
        ] {
            Digest::nonzero(digest.0)?;
        }
        if self.work == [0; 32] {
            return Err(zutti_protocol::Error::Invalid("SPV window work"));
        }
        writer.fixed(b"ZUTTISPW");
        writer.u16(1);
        for digest in [self.policy, self.prefix, self.reservation] {
            writer.fixed(&digest.0);
        }
        writer.u32(self.height);
        writer.fixed(&self.tip.0);
        writer.fixed(&self.work);
        writer.fixed(&self.output.0);
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTISPW" || reader.u16()? != 1 {
            return Err(zutti_protocol::Error::Version);
        }
        let claim = Self {
            policy: Digest(reader.fixed()?),
            prefix: Digest(reader.fixed()?),
            reservation: Digest(reader.fixed()?),
            height: reader.u32()?,
            tip: Digest(reader.fixed()?),
            work: reader.fixed()?,
            output: Digest(reader.fixed()?),
        };
        claim.to_bytes()?;
        Ok(claim)
    }
}
