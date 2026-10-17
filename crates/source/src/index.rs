use crate::{Error, Result};
use std::collections::BTreeMap;
use zutti_protocol::{
    Digest,
    commitment::hash,
    encoding::{Canonical, Reader, Writer},
};

const DEPTH: usize = 256;
const MAX_TRANSACTIONS: usize = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexContext {
    genesis: Digest,
    rules: Digest,
    start: u32,
}
impl IndexContext {
    pub fn new(genesis: Digest, rules: Digest, start: u32) -> Result<Self> {
        Digest::nonzero(genesis.0)?;
        Digest::nonzero(rules.0)?;
        Ok(Self {
            genesis,
            rules,
            start,
        })
    }
    pub fn id(self) -> Digest {
        hash(
            b"index-context",
            &[&self.genesis.0, &self.rules.0, &self.start.to_le_bytes()],
        )
    }
    pub const fn start(self) -> u32 {
        self.start
    }
    pub const fn genesis(self) -> Digest {
        self.genesis
    }
    pub const fn rules(self) -> Digest {
        self.rules
    }
}

impl Canonical for IndexContext {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        writer.fixed(&self.genesis.0);
        writer.fixed(&self.rules.0);
        writer.u32(self.start);
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        Self::new(
            Digest(reader.fixed()?),
            Digest(reader.fixed()?),
            reader.u32()?,
        )
        .map_err(|_| zutti_protocol::Error::Invalid("index context"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexSnapshot {
    pub context: IndexContext,
    pub height: u32,
    pub root: Digest,
}

impl Canonical for IndexSnapshot {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        if self.height < self.context.start() {
            return Err(zutti_protocol::Error::Invalid("index height"));
        }
        Digest::nonzero(self.root.0)?;
        self.context.encode(writer)?;
        writer.u32(self.height);
        writer.fixed(&self.root.0);
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        let value = Self {
            context: IndexContext::decode(reader)?,
            height: reader.u32()?,
            root: Digest::nonzero(reader.fixed()?)?,
        };
        if value.height < value.context.start() {
            return Err(zutti_protocol::Error::Invalid("index height"));
        }
        Ok(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Observation {
    Present(u32),
    Absent,
}

#[derive(Clone, PartialEq, Eq)]
pub struct IndexProof {
    context: Digest,
    key: Digest,
    first_height: Option<u32>,
    siblings: [Digest; DEPTH],
}
impl core::fmt::Debug for IndexProof {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("IndexProof([PRIVATE QUERY])")
    }
}
impl IndexProof {
    pub fn verify(&self, snapshot: IndexSnapshot, key: Digest) -> Result<Observation> {
        if self.key != key
            || self.context != snapshot.context.id()
            || snapshot.height < snapshot.context.start
        {
            return Err(Error::Path);
        }
        let mut node = match self.first_height {
            Some(height) if height > snapshot.context.start && height <= snapshot.height => {
                leaf(self.context, key, height)
            }
            Some(_) => return Err(Error::Path),
            None => empty_leaf(self.context),
        };
        for (level, sibling) in self.siblings.iter().enumerate() {
            let depth = DEPTH - 1 - level;
            node = if bit(key, depth) {
                branch(self.context, depth, *sibling, node)
            } else {
                branch(self.context, depth, node, *sibling)
            };
        }
        if node != snapshot.root {
            return Err(Error::Path);
        }
        Ok(self
            .first_height
            .map_or(Observation::Absent, Observation::Present))
    }
}
impl Canonical for IndexProof {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        writer.fixed(b"ZUTTIPTH");
        writer.u16(1);
        writer.fixed(&self.context.0);
        writer.fixed(&self.key.0);
        match self.first_height {
            Some(h) => {
                writer.u8(1);
                writer.u32(h);
            }
            None => writer.u8(0),
        }
        for sibling in self.siblings {
            writer.fixed(&sibling.0);
        }
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTIPTH" || reader.u16()? != 1 {
            return Err(zutti_protocol::Error::Version);
        }
        let context = Digest(reader.fixed()?);
        let key = Digest(reader.fixed()?);
        let first_height = match reader.u8()? {
            0 => None,
            1 => Some(reader.u32()?),
            _ => return Err(zutti_protocol::Error::Invalid("path presence flag")),
        };
        let mut siblings = [Digest::default(); DEPTH];
        for sibling in &mut siblings {
            *sibling = Digest(reader.fixed()?);
        }
        Ok(Self {
            context,
            key,
            first_height,
            siblings,
        })
    }
}

#[derive(Debug)]
pub struct CandidateIndex {
    context: IndexContext,
    height: u32,
    entries: BTreeMap<Digest, u32>,
    nodes: BTreeMap<(usize, Digest), Digest>,
    empty: [Digest; DEPTH + 1],
}
impl CandidateIndex {
    pub fn new(context: IndexContext) -> Self {
        let id = context.id();
        let mut empty = [Digest::default(); DEPTH + 1];
        empty[DEPTH] = empty_leaf(id);
        for depth in (0..DEPTH).rev() {
            empty[depth] = branch(id, depth, empty[depth + 1], empty[depth + 1]);
        }
        Self {
            context,
            height: context.start,
            entries: BTreeMap::new(),
            nodes: BTreeMap::new(),
            empty,
        }
    }
    pub fn snapshot(&self) -> IndexSnapshot {
        IndexSnapshot {
            context: self.context,
            height: self.height,
            root: self.node(0, Digest::default()),
        }
    }
    pub fn append_block(&mut self, height: u32, transaction_ids: &[Digest]) -> Result<()> {
        if self.height.checked_add(1) != Some(height) {
            return Err(Error::Height);
        }
        if transaction_ids.is_empty()
            || transaction_ids.len() > MAX_TRANSACTIONS
            || transaction_ids.iter().any(|t| t.0 == [0; 32])
        {
            return Err(Error::Transactions);
        }
        // Validate the entire batch before mutating. Preserve first inclusion deterministically.
        for key in transaction_ids {
            if self.entries.contains_key(key) {
                continue;
            }
            self.entries.insert(*key, height);
            let id = self.context.id();
            let mut current = leaf(id, *key, height);
            self.nodes.insert((DEPTH, *key), current);
            for depth in (0..DEPTH).rev() {
                let sibling = self.node(depth + 1, sibling_prefix(*key, depth));
                current = if bit(*key, depth) {
                    branch(id, depth, sibling, current)
                } else {
                    branch(id, depth, current, sibling)
                };
                self.nodes.insert((depth, prefix(*key, depth)), current);
            }
        }
        self.height = height;
        Ok(())
    }
    pub fn prove(&self, key: Digest) -> IndexProof {
        let mut siblings = [Digest::default(); DEPTH];
        for (level, sibling) in siblings.iter_mut().enumerate() {
            let depth = DEPTH - 1 - level;
            *sibling = self.node(depth + 1, sibling_prefix(key, depth));
        }
        IndexProof {
            context: self.context.id(),
            key,
            first_height: self.entries.get(&key).copied(),
            siblings,
        }
    }
    fn node(&self, depth: usize, key: Digest) -> Digest {
        self.nodes
            .get(&(depth, key))
            .copied()
            .unwrap_or(self.empty[depth])
    }
}

fn bit(key: Digest, depth: usize) -> bool {
    key.0[depth / 8] & (1 << (7 - depth % 8)) != 0
}
fn prefix(key: Digest, depth: usize) -> Digest {
    let mut out = key.0;
    let full = depth / 8;
    let partial = depth % 8;
    if full < 32 {
        if partial == 0 {
            out[full..].fill(0);
        } else {
            out[full] &= 0xff << (8 - partial);
            out[full + 1..].fill(0);
        }
    }
    Digest(out)
}
fn sibling_prefix(key: Digest, depth: usize) -> Digest {
    let mut out = prefix(key, depth + 1).0;
    out[depth / 8] ^= 1 << (7 - depth % 8);
    Digest(out)
}
fn empty_leaf(context: Digest) -> Digest {
    hash(b"index-empty", &[&context.0])
}
fn leaf(context: Digest, key: Digest, height: u32) -> Digest {
    hash(b"index-leaf", &[&context.0, &key.0, &height.to_le_bytes()])
}
fn branch(context: Digest, depth: usize, left: Digest, right: Digest) -> Digest {
    hash(
        b"index-branch",
        &[&context.0, &(depth as u16).to_le_bytes(), &left.0, &right.0],
    )
}
