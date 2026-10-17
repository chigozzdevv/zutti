use crate::{
    IndexSnapshot,
    authenticated::{Access, StateError},
};
use zutti_protocol::{
    Digest, Error,
    commitment::hash,
    encoding::{Canonical, Reader, Writer},
};

pub(crate) fn context(genesis: Digest, policy: Digest) -> Digest {
    hash(b"source-header-archive", &[&genesis.0, &policy.0])
}
pub(crate) fn key(height: u32) -> Digest {
    hash(b"source-header-height", &[&height.to_le_bytes()])
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderRecord {
    pub height: u32,
    pub hash: Digest,
    pub work: [u8; 32],
    pub index: IndexSnapshot,
}
impl Canonical for HeaderRecord {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        Digest::nonzero(self.hash.0)?;
        if self.work == [0; 32] || self.index.height != self.height.max(self.index.context.start())
        {
            return Err(Error::Invalid("archived header"));
        }
        writer.u32(self.height);
        writer.fixed(&self.hash.0);
        writer.fixed(&self.work);
        self.index.encode(writer)
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        let record = Self {
            height: reader.u32()?,
            hash: Digest(reader.fixed()?),
            work: reader.fixed()?,
            index: IndexSnapshot::decode(reader)?,
        };
        record.to_bytes()?;
        Ok(record)
    }
}

#[derive(Debug, Clone)]
pub struct HeaderPath {
    pub access: Access,
}
impl HeaderPath {
    pub fn verify(
        &self,
        genesis: Digest,
        policy: Digest,
        root: Digest,
        height: u32,
    ) -> Result<HeaderRecord, StateError> {
        let bytes = self
            .access
            .verify(context(genesis, policy), root, key(height))?
            .ok_or(StateError::Access)?;
        let record = HeaderRecord::from_bytes(&bytes)?;
        if record.height != height
            || record.index.context.genesis() != genesis
            || record.index.context.rules() != policy
        {
            return Err(StateError::Access);
        }
        Ok(record)
    }
}
impl Canonical for HeaderPath {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        self.access.encode(writer)
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        Ok(Self {
            access: Access::decode(reader)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCheckpoint {
    pub stable: HeaderRecord,
    pub archive: Digest,
}
impl SourceCheckpoint {
    pub fn id(&self) -> zutti_protocol::Result<Digest> {
        Ok(hash(b"source-checkpoint", &[&self.to_bytes()?]))
    }
    pub fn verify_anchor(
        &self,
        height: u32,
        hash: Digest,
        path: &HeaderPath,
    ) -> Result<(), StateError> {
        if height > self.stable.height {
            return Err(StateError::Access);
        }
        let context = self.stable.index.context;
        let header = path.verify(context.genesis(), context.rules(), self.archive, height)?;
        if header.hash != hash {
            return Err(StateError::Access);
        }
        Ok(())
    }
}
impl Canonical for SourceCheckpoint {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        writer.fixed(b"ZUTTICKP");
        writer.u16(1);
        self.stable.encode(writer)?;
        writer.fixed(&Digest::nonzero(self.archive.0)?.0);
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTICKP" || reader.u16()? != 1 {
            return Err(Error::Version);
        }
        Ok(Self {
            stable: HeaderRecord::decode(reader)?,
            archive: Digest::nonzero(reader.fixed()?)?,
        })
    }
}
