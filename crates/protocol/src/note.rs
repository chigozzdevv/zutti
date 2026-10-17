use crate::{
    Digest, Error, Result,
    amount::{Amount, Asset},
    encoding::{Canonical, Reader, Writer},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedOutput {
    commitment: Digest,
    ciphertext_digest: Digest,
    amount: Amount,
}
impl FixedOutput {
    pub fn new(commitment: Digest, ciphertext_digest: Digest, amount: Amount) -> Result<Self> {
        Digest::nonzero(commitment.0)?;
        Digest::nonzero(ciphertext_digest.0)?;
        amount.positive()?;
        if amount.asset() != Asset::Usdc {
            return Err(Error::Asset);
        }
        Ok(Self {
            commitment,
            ciphertext_digest,
            amount,
        })
    }
    pub const fn commitment(self) -> Digest {
        self.commitment
    }
    pub const fn ciphertext_digest(self) -> Digest {
        self.ciphertext_digest
    }
    pub const fn amount(self) -> Amount {
        self.amount
    }
}
impl Canonical for FixedOutput {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.fixed(&self.commitment.0);
        writer.fixed(&self.ciphertext_digest.0);
        self.amount.encode(writer)
    }
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Self::new(
            Digest(reader.fixed()?),
            Digest(reader.fixed()?),
            Amount::decode(reader)?,
        )
    }
}
