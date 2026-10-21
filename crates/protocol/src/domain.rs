use crate::{
    Digest, Error, Result,
    commitment::hash,
    encoding::{Canonical, Reader, Writer},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Domain {
    source_genesis: Digest,
    destination_chain: u64,
    pool: [u8; 20],
    token: [u8; 20],
    payment_profile: Digest,
    source_policy: Digest,
    proof_profile: Digest,
}
impl Domain {
    pub fn new(
        source_genesis: Digest,
        destination_chain: u64,
        pool: [u8; 20],
        token: [u8; 20],
        payment_profile: Digest,
        source_policy: Digest,
        proof_profile: Digest,
    ) -> Result<Self> {
        if destination_chain == 0 || pool == [0; 20] || token == [0; 20] || pool == token {
            return Err(Error::Invalid("deployment domain"));
        }
        for value in [
            source_genesis,
            payment_profile,
            source_policy,
            proof_profile,
        ] {
            Digest::nonzero(value.0)?;
        }
        Ok(Self {
            source_genesis,
            destination_chain,
            pool,
            token,
            payment_profile,
            source_policy,
            proof_profile,
        })
    }
    pub fn id(&self) -> Result<Digest> {
        Ok(hash(b"domain", &[&self.to_bytes()?]))
    }
    pub const fn source_genesis(&self) -> Digest {
        self.source_genesis
    }
    pub const fn source_policy(&self) -> Digest {
        self.source_policy
    }
    pub const fn payment_profile(&self) -> Digest {
        self.payment_profile
    }
    pub const fn proof_profile(&self) -> Digest {
        self.proof_profile
    }
    pub const fn pool(&self) -> [u8; 20] {
        self.pool
    }
    pub const fn destination_chain(&self) -> u64 {
        self.destination_chain
    }
}
impl Canonical for Domain {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.fixed(b"ZUTTIDOM");
        writer.u16(1);
        writer.fixed(&self.source_genesis.0);
        writer.u64(self.destination_chain);
        writer.fixed(&self.pool);
        writer.fixed(&self.token);
        writer.fixed(&self.payment_profile.0);
        writer.fixed(&self.source_policy.0);
        writer.fixed(&self.proof_profile.0);
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTIDOM" || reader.u16()? != 1 {
            return Err(Error::Version);
        }
        Self::new(
            Digest(reader.fixed()?),
            reader.u64()?,
            reader.fixed()?,
            reader.fixed()?,
            Digest(reader.fixed()?),
            Digest(reader.fixed()?),
            Digest(reader.fixed()?),
        )
    }
}
