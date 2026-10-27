use crate::{
    Error, Result,
    private_note::{EncryptedNote, NoteKeys},
};
use zutti_protocol::{
    Digest,
    encoding::{Canonical, Reader, Writer},
    pool::{PoolPath, PoolTree},
    private_note::PrivateNote,
};

pub struct OwnedNote {
    pub note: PrivateNote,
    pub path: PoolPath,
}

impl Canonical for PoolInventory {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        if self.notes.len() > 64 {
            return Err(zutti_protocol::Error::Length);
        }
        writer.fixed(b"ZUTTIINV");
        writer.u16(1);
        writer.fixed(&self.domain.0);
        writer.fixed(&self.root.0);
        writer.u32(self.size);
        writer.u32(self.notes.len() as u32);
        for owned in &self.notes {
            owned.note.encode(writer)?;
            owned.path.encode(writer)?;
        }
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTIINV" || reader.u16()? != 1 {
            return Err(zutti_protocol::Error::Version);
        }
        let domain = Digest::nonzero(reader.fixed()?)?;
        let root = Digest::nonzero(reader.fixed()?)?;
        let size = reader.u32()?;
        let count = reader.u32()?;
        if count > 64 || count > size {
            return Err(zutti_protocol::Error::Length);
        }
        let mut positions = std::collections::BTreeSet::new();
        let mut notes = Vec::new();
        for _ in 0..count {
            let note = PrivateNote::decode(reader)?;
            let path = PoolPath::decode(reader)?;
            if note.domain() != domain || path.position >= size || !positions.insert(path.position)
            {
                return Err(zutti_protocol::Error::Invalid("inventory ownership path"));
            }
            path.verify(domain, note.commitment()?, root)?;
            notes.push(OwnedNote { note, path });
        }
        Ok(Self {
            domain,
            root,
            size,
            notes,
        })
    }
}
impl core::fmt::Debug for OwnedNote {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("OwnedNote([REDACTED])")
    }
}

pub struct PoolInventory {
    domain: Digest,
    root: Digest,
    size: u32,
    notes: Vec<OwnedNote>,
}

impl core::fmt::Debug for PoolInventory {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PoolInventory")
            .field("size", &self.size)
            .field("owned_notes", &self.notes.len())
            .finish_non_exhaustive()
    }
}

impl PoolInventory {
    pub fn scan(
        domain: Digest,
        keys: &NoteKeys,
        outputs: &[(u32, EncryptedNote)],
        expected_root: Digest,
        expected_size: u32,
    ) -> Result<Self> {
        if outputs.len() > 65_536 || outputs.len() != expected_size as usize {
            return Err(Error::Archive);
        }
        let mut tree = PoolTree::new(domain)?;
        let recipient = keys.recipient()?;
        let mut owned = Vec::new();
        for (position, encrypted) in outputs {
            if encrypted.domain() != domain
                || u64::from(*position) != tree.size()
                || tree.append(encrypted.commitment())? != *position
            {
                return Err(Error::Archive);
            }
            if let Ok(note) = keys.decrypt(encrypted, domain) {
                if note.recipient() != recipient {
                    return Err(Error::Archive);
                }
                owned.push((*position, note));
            }
        }
        if tree.root() != expected_root {
            return Err(Error::Archive);
        }
        let notes = owned
            .into_iter()
            .map(|(position, note)| {
                let path = tree.prove(position)?;
                path.verify(domain, note.commitment()?, expected_root)?;
                Ok(OwnedNote { note, path })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            domain,
            root: expected_root,
            size: expected_size,
            notes,
        })
    }

    pub const fn root(&self) -> Digest {
        self.root
    }

    pub const fn size(&self) -> u32 {
        self.size
    }

    pub fn notes(&self) -> &[OwnedNote] {
        &self.notes
    }
}
