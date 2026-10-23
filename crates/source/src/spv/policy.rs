use zcash_protocol::consensus::NetworkType;
use zutti_protocol::{
    Digest, Error,
    commitment::hash,
    encoding::{Canonical, Reader, Writer},
};

pub const MAX_HEADERS: usize = 4096;
pub const MAX_WINDOW_BLOCKS: u32 = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    program: Digest,
    network: NetworkType,
    confirmations: u32,
    window_blocks: u32,
    valid_through: u32,
}

impl Policy {
    pub fn new(
        program: Digest,
        network: NetworkType,
        confirmations: u32,
        window_blocks: u32,
        valid_through: u32,
    ) -> zutti_protocol::Result<Self> {
        let policy = Self {
            program,
            network,
            confirmations,
            window_blocks,
            valid_through,
        };
        policy.to_bytes()?;
        Ok(policy)
    }
    pub fn id(self) -> zutti_protocol::Result<Digest> {
        Ok(hash(b"spv-policy", &[&self.to_bytes()?]))
    }
    pub const fn program(self) -> Digest {
        self.program
    }
    pub const fn network(self) -> NetworkType {
        self.network
    }
    pub const fn confirmations(self) -> u32 {
        self.confirmations
    }
    pub const fn window_blocks(self) -> u32 {
        self.window_blocks
    }
    pub const fn valid_through(self) -> u32 {
        self.valid_through
    }
}

impl Canonical for Policy {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        Digest::nonzero(self.program.0)?;
        if self.confirmations == 0
            || self.confirmations > 1000
            || self.window_blocks == 0
            || self.window_blocks > MAX_WINDOW_BLOCKS
            || self.valid_through == 0
            || self.valid_through >= 500_000_000
        {
            return Err(Error::Invalid("SPV policy bounds"));
        }
        writer.fixed(b"ZUTTISPP");
        writer.u16(1);
        writer.fixed(&self.program.0);
        writer.u8(match self.network {
            NetworkType::Main => 1,
            NetworkType::Test => 2,
            _ => return Err(Error::Invalid("SPV network")),
        });
        writer.u32(self.confirmations);
        writer.u32(self.window_blocks);
        writer.u32(self.valid_through);
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTISPP" || reader.u16()? != 1 {
            return Err(Error::Version);
        }
        let program = Digest(reader.fixed()?);
        let network = match reader.u8()? {
            1 => NetworkType::Main,
            2 => NetworkType::Test,
            _ => return Err(Error::Invalid("SPV network")),
        };
        Self::new(
            program,
            network,
            reader.u32()?,
            reader.u32()?,
            reader.u32()?,
        )
    }
}
