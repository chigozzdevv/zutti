use std::cell::RefCell;
use zutti_protocol::{
    Digest, Error,
    commitment::hash,
    encoding::{Canonical, Reader, Writer},
};

const DEPTH: usize = 256;
const MAX_VALUE: usize = 16_384;
const MAX_ACCESSES: usize = 1_000_000;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum StateError {
    #[error(transparent)]
    Encoding(#[from] Error),
    #[error("authenticated state access does not match its root or requested key")]
    Access,
    #[error("authenticated state witness has missing or unused accesses")]
    Transcript,
}
pub type Result<T> = core::result::Result<T, StateError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Access {
    key: Digest,
    value: Option<Vec<u8>>,
    bitmap: [u8; 32],
    siblings: Vec<Digest>,
}
impl Access {
    #[cfg(feature = "storage")]
    pub(crate) const fn key(&self) -> Digest {
        self.key
    }

    pub fn verify(&self, context: Digest, root: Digest, key: Digest) -> Result<Option<Vec<u8>>> {
        AuthenticatedMap::from_witness(context, root, Transcript(vec![self.clone()])).get(key)
    }
}
impl Canonical for Access {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        writer.fixed(&self.key.0);
        match &self.value {
            None => writer.u8(0),
            Some(value) if !value.is_empty() && value.len() <= MAX_VALUE => {
                writer.u8(1);
                writer.bytes(value)?;
            }
            Some(_) => return Err(Error::Length),
        }
        if self
            .bitmap
            .iter()
            .map(|b| b.count_ones() as usize)
            .sum::<usize>()
            != self.siblings.len()
        {
            return Err(Error::Length);
        }
        writer.fixed(&self.bitmap);
        for sibling in &self.siblings {
            writer.fixed(&sibling.0);
        }
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        let key = Digest(reader.fixed()?);
        let value = match reader.u8()? {
            0 => None,
            1 => {
                let bytes = reader.bytes(MAX_VALUE)?;
                if bytes.is_empty() {
                    return Err(Error::Length);
                }
                Some(bytes.to_vec())
            }
            _ => return Err(Error::Invalid("state presence flag")),
        };
        let bitmap: [u8; 32] = reader.fixed()?;
        let count = bitmap.iter().map(|b| b.count_ones() as usize).sum();
        let mut siblings = Vec::with_capacity(count);
        for _ in 0..count {
            siblings.push(Digest(reader.fixed()?));
        }
        Ok(Self {
            key,
            value,
            bitmap,
            siblings,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transcript(Vec<Access>);
impl Transcript {
    #[cfg(feature = "storage")]
    pub(crate) fn accesses(&self) -> &[Access] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
impl Canonical for Transcript {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        if self.0.len() > MAX_ACCESSES {
            return Err(Error::Length);
        }
        writer.fixed(b"ZUTTIACC");
        writer.u16(1);
        writer.u32(self.0.len() as u32);
        for access in &self.0 {
            access.encode(writer)?;
        }
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTIACC" || reader.u16()? != 1 {
            return Err(Error::Version);
        }
        let count = reader.u32()? as usize;
        if count > MAX_ACCESSES {
            return Err(Error::Length);
        }
        let mut accesses = Vec::new();
        for _ in 0..count {
            accesses.push(Access::decode(reader)?);
        }
        Ok(Self(accesses))
    }
}

#[derive(Debug, Clone)]
enum Backend {
    Full {
        values: im::OrdMap<Digest, Vec<u8>>,
        nodes: crate::sparse::SparseTree,
        recording: RefCell<Option<Vec<Access>>>,
    },
    Witness {
        accesses: Vec<Access>,
        cursor: RefCell<usize>,
    },
}

#[derive(Debug, Clone)]
pub struct AuthenticatedMap {
    context: Digest,
    root: Digest,
    empty: [Digest; DEPTH + 1],
    backend: Backend,
}
impl AuthenticatedMap {
    pub fn new(context: Digest) -> Self {
        let empty = empty_nodes(context);
        Self {
            context,
            root: empty[0],
            empty,
            backend: Backend::Full {
                values: im::OrdMap::new(),
                nodes: crate::sparse::SparseTree::default(),
                recording: RefCell::new(None),
            },
        }
    }
    #[cfg(feature = "storage")]
    pub(crate) fn stored_values(&self) -> Result<impl Iterator<Item = (&Digest, &Vec<u8>)>> {
        match &self.backend {
            Backend::Full {
                values, recording, ..
            } if recording.borrow().is_none() => Ok(values.iter()),
            _ => Err(StateError::Transcript),
        }
    }
    pub fn from_witness(context: Digest, root: Digest, transcript: Transcript) -> Self {
        Self {
            context,
            root,
            empty: empty_nodes(context),
            backend: Backend::Witness {
                accesses: transcript.0,
                cursor: RefCell::new(0),
            },
        }
    }
    pub const fn root(&self) -> Digest {
        self.root
    }
    pub fn len(&self) -> Option<usize> {
        match &self.backend {
            Backend::Full { values, .. } => Some(values.len()),
            Backend::Witness { .. } => None,
        }
    }
    pub fn is_empty(&self) -> Option<bool> {
        self.len().map(|n| n == 0)
    }
    pub fn start_recording(&mut self) -> Result<()> {
        let Backend::Full { recording, .. } = &self.backend else {
            return Err(StateError::Transcript);
        };
        *recording.borrow_mut() = Some(Vec::new());
        Ok(())
    }
    pub fn take_transcript(&mut self) -> Result<Transcript> {
        let Backend::Full { recording, .. } = &self.backend else {
            return Err(StateError::Transcript);
        };
        recording
            .borrow_mut()
            .take()
            .map(Transcript)
            .ok_or(StateError::Transcript)
    }
    pub fn finish(&self) -> Result<()> {
        if let Backend::Witness { accesses, cursor } = &self.backend
            && *cursor.borrow() != accesses.len()
        {
            return Err(StateError::Transcript);
        }
        Ok(())
    }
    pub fn prove(&self, key: Digest) -> Result<Access> {
        self.access(key)
    }
    pub fn get(&self, key: Digest) -> Result<Option<Vec<u8>>> {
        if let Backend::Full {
            values, recording, ..
        } = &self.backend
            && recording.borrow().is_none()
        {
            return Ok(values.get(&key).cloned());
        }
        Ok(self.access(key)?.value)
    }
    pub fn set(&mut self, key: Digest, value: Option<Vec<u8>>) -> Result<Option<Vec<u8>>> {
        if value
            .as_ref()
            .is_some_and(|v| v.is_empty() || v.len() > MAX_VALUE)
        {
            return Err(Error::Length.into());
        }
        let leaf = self.leaf(key, value.as_deref());
        if matches!(self.backend, Backend::Full { .. }) {
            let previous = self.get(key)?;
            let Backend::Full { values, nodes, .. } = &mut self.backend else {
                return Err(StateError::Transcript);
            };
            let context = self.context;
            nodes.set(
                key,
                value.as_ref().map(|_| leaf),
                &self.empty,
                &|depth, left, right| branch(context, depth, left, right),
            );
            match value {
                Some(value) => {
                    values.insert(key, value);
                }
                None => {
                    values.remove(&key);
                }
            }
            self.root = nodes.root(&self.empty);
            return Ok(previous);
        }
        let access = self.access(key)?;
        let path = self.expand(&access)?;
        let mut current = leaf;
        for (offset, sibling) in path.iter().enumerate() {
            current = self.join(DEPTH - 1 - offset, key, current, *sibling);
        }
        self.root = current;
        Ok(access.value)
    }
    fn access(&self, key: Digest) -> Result<Access> {
        match &self.backend {
            Backend::Full {
                values,
                nodes,
                recording,
            } => {
                let mut bitmap = [0; 32];
                let mut siblings = Vec::new();
                let path = nodes.prove(key, &self.empty, &|depth, left, right| {
                    branch(self.context, depth, left, right)
                });
                for (offset, sibling) in path.into_iter().enumerate() {
                    let depth = DEPTH - 1 - offset;
                    if sibling != self.empty[depth + 1] {
                        bitmap[offset / 8] |= 1 << (offset % 8);
                        siblings.push(sibling);
                    }
                }
                let access = Access {
                    key,
                    value: values.get(&key).cloned(),
                    bitmap,
                    siblings,
                };
                if let Some(log) = recording.borrow_mut().as_mut() {
                    if log.len() == MAX_ACCESSES {
                        return Err(StateError::Transcript);
                    }
                    log.push(access.clone());
                }
                Ok(access)
            }
            Backend::Witness { accesses, cursor } => {
                let mut index = cursor.borrow_mut();
                let access = accesses.get(*index).ok_or(StateError::Transcript)?;
                if access.key != key {
                    return Err(StateError::Access);
                }
                let mut current = self.leaf(key, access.value.as_deref());
                for (offset, sibling) in self.expand(access)?.iter().enumerate() {
                    current = self.join(DEPTH - 1 - offset, key, current, *sibling);
                }
                if current != self.root {
                    return Err(StateError::Access);
                }
                *index += 1;
                Ok(access.clone())
            }
        }
    }
    fn leaf(&self, key: Digest, value: Option<&[u8]>) -> Digest {
        value.map_or(self.empty[DEPTH], |value| {
            hash(b"state-leaf", &[&self.context.0, &key.0, value])
        })
    }
    fn join(&self, depth: usize, key: Digest, value: Digest, sibling: Digest) -> Digest {
        let (left, right) = if key.0[depth / 8] & (1 << (7 - depth % 8)) == 0 {
            (value, sibling)
        } else {
            (sibling, value)
        };
        branch(self.context, depth, left, right)
    }
    fn expand(&self, access: &Access) -> Result<[Digest; DEPTH]> {
        let mut path = [Digest::default(); DEPTH];
        let mut cursor = 0;
        for (offset, sibling) in path.iter_mut().enumerate() {
            let default = self.empty[DEPTH - offset];
            *sibling = if access.bitmap[offset / 8] & (1 << (offset % 8)) == 0 {
                default
            } else {
                let value = *access.siblings.get(cursor).ok_or(StateError::Access)?;
                if value == default {
                    return Err(StateError::Access);
                }
                cursor += 1;
                value
            };
        }
        if cursor != access.siblings.len() {
            return Err(StateError::Access);
        }
        Ok(path)
    }
}

fn branch(context: Digest, depth: usize, left: Digest, right: Digest) -> Digest {
    hash(
        b"state-branch",
        &[&context.0, &(depth as u16).to_le_bytes(), &left.0, &right.0],
    )
}
fn empty_nodes(context: Digest) -> [Digest; DEPTH + 1] {
    let mut nodes = [Digest::default(); DEPTH + 1];
    nodes[DEPTH] = hash(b"state-empty", &[&context.0]);
    for depth in (0..DEPTH).rev() {
        nodes[depth] = branch(context, depth, nodes[depth + 1], nodes[depth + 1]);
    }
    nodes
}
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    fn exercise(map: &mut AuthenticatedMap) {
        assert_eq!(map.get(Digest([1; 32])).unwrap(), None);
        assert_eq!(map.set(Digest([1; 32]), Some(vec![7])).unwrap(), None);
        assert_eq!(map.set(Digest([2; 32]), Some(vec![8])).unwrap(), None);
        assert_eq!(map.get(Digest([1; 32])).unwrap(), Some(vec![7]));
        assert_eq!(map.set(Digest([1; 32]), None).unwrap(), Some(vec![7]));
        assert_eq!(map.get(Digest([1; 32])).unwrap(), None);
    }
    #[test]
    fn streamed_accesses_replay_exactly_and_reject_tampering() {
        let context = Digest([9; 32]);
        let mut full = AuthenticatedMap::new(context);
        let root = full.root();
        full.start_recording().unwrap();
        exercise(&mut full);
        let log = full.take_transcript().unwrap();
        let bytes = log.to_bytes().unwrap();
        assert!(bytes.len() < 1_000);
        let mut replay =
            AuthenticatedMap::from_witness(context, root, Transcript::from_bytes(&bytes).unwrap());
        exercise(&mut replay);
        replay.finish().unwrap();
        assert_eq!(replay.root(), full.root());
        assert!(replay.get(Digest([2; 32])).is_err());
        let mut corrupt = log.clone();
        corrupt.0[3].value = Some(vec![99]);
        let mut replay = AuthenticatedMap::from_witness(context, root, corrupt);
        assert!(replay.get(Digest([2; 32])).is_err());
        assert!(replay.finish().is_err());
        assert!(replay.get(Digest([1; 32])).unwrap().is_none());
        replay.set(Digest([1; 32]), Some(vec![7])).unwrap();
        replay.set(Digest([2; 32]), Some(vec![8])).unwrap();
        assert!(replay.get(Digest([1; 32])).is_err());
        let wrong = AuthenticatedMap::from_witness(Digest([8; 32]), root, log);
        assert!(wrong.get(Digest([1; 32])).is_err());
    }
    #[test]
    fn deletions_restore_empty_root_and_branches_do_not_share_mutation() {
        let mut map = AuthenticatedMap::new(Digest([3; 32]));
        let root = map.root();
        for byte in 0..100 {
            map.set(Digest([byte; 32]), Some(vec![byte])).unwrap();
        }
        let fork = map.clone();
        for byte in 0..100 {
            map.set(Digest([byte; 32]), None).unwrap();
        }
        assert_eq!(map.root(), root);
        assert_eq!(map.len(), Some(0));
        assert_eq!(fork.len(), Some(100));
        assert_ne!(fork.root(), root);
    }
    #[test]
    fn empty_values_and_noncanonical_transcripts_are_rejected() {
        let mut map = AuthenticatedMap::new(Digest([3; 32]));
        assert!(map.set(Digest([1; 32]), Some(Vec::new())).is_err());
        map.start_recording().unwrap();
        map.get(Digest([1; 32])).unwrap();
        let mut bytes = map.take_transcript().unwrap().to_bytes().unwrap();
        bytes.push(0);
        assert!(Transcript::from_bytes(&bytes).is_err());
    }
}
