use crate::{
    Digest, Error, Result,
    encoding::{Canonical, Reader, Writer},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaymentWindow {
    index_start: u32,
    anchor_height: u32,
    anchor_hash: Digest,
    expiry: u32,
}
impl PaymentWindow {
    pub fn new(
        index_start: u32,
        anchor_height: u32,
        anchor_hash: Digest,
        expiry: u32,
    ) -> Result<Self> {
        Digest::nonzero(anchor_hash.0)?;
        if anchor_height <= index_start || expiry <= anchor_height || expiry >= 500_000_000 {
            return Err(Error::Invalid("payment window"));
        }
        Ok(Self {
            index_start,
            anchor_height,
            anchor_hash,
            expiry,
        })
    }
    pub const fn index_start(self) -> u32 {
        self.index_start
    }
    pub const fn anchor_height(self) -> u32 {
        self.anchor_height
    }
    pub const fn anchor_hash(self) -> Digest {
        self.anchor_hash
    }
    pub const fn expiry(self) -> u32 {
        self.expiry
    }
    pub fn require_remaining(self, observed_height: u32, minimum_blocks: u32) -> Result<()> {
        if minimum_blocks == 0
            || observed_height < self.anchor_height
            || observed_height
                .checked_add(minimum_blocks)
                .is_none_or(|h| h > self.expiry)
        {
            return Err(Error::Invalid("remaining inclusion window"));
        }
        Ok(())
    }
}
impl Canonical for PaymentWindow {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.u32(self.index_start);
        writer.u32(self.anchor_height);
        writer.fixed(&self.anchor_hash.0);
        writer.u32(self.expiry);
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Self::new(
            reader.u32()?,
            reader.u32()?,
            Digest(reader.fixed()?),
            reader.u32()?,
        )
    }
}
