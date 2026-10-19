use crate::chain_history::CommitmentTreeRoots;
mod empty;
use incrementalmerkletree::{Hashable, Level, Position, frontier::Frontier};
use orchard::tree::MerkleHashOrchard;
use sapling::Node as SaplingNode;
use std::sync::OnceLock;
use zcash_primitives::transaction::Transaction;
use zutti_protocol::{
    Digest, Error as CodecError,
    commitment::hash,
    encoding::{Canonical, Reader, Writer},
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("shielded commitment tree is full")]
    Full,
    #[error("invalid shielded note commitment")]
    Commitment,
    #[error("too many note commitments in a block")]
    Limit,
    #[error(transparent)]
    Encoding(#[from] CodecError),
}

pub type Result<T> = core::result::Result<T, Error>;

#[derive(Debug, Clone, Default)]
pub struct CommitmentUpdates {
    sprout: Vec<Digest>,
    sapling: Vec<Digest>,
    orchard: Vec<Digest>,
    ironwood: Vec<Digest>,
}

impl CommitmentUpdates {
    pub fn sprout(&self) -> &[Digest] {
        &self.sprout
    }
    pub fn sapling(&self) -> &[Digest] {
        &self.sapling
    }
    pub fn orchard(&self) -> &[Digest] {
        &self.orchard
    }
    pub fn ironwood(&self) -> &[Digest] {
        &self.ironwood
    }
    pub(crate) fn include(&mut self, transaction: &Transaction) {
        if let Some(bundle) = transaction.sprout_bundle() {
            for joinsplit in &bundle.joinsplits {
                self.sprout
                    .extend(joinsplit.commitments().iter().copied().map(Digest));
            }
        }
        if let Some(bundle) = transaction.sapling_bundle() {
            self.sapling.extend(
                bundle
                    .shielded_outputs()
                    .iter()
                    .map(|output| Digest(output.cmu().to_bytes())),
            );
        }
        if let Some(bundle) = transaction.orchard_bundle() {
            self.orchard.extend(
                bundle
                    .actions()
                    .iter()
                    .map(|action| Digest(action.cmx().to_bytes())),
            );
        }
        if let Some(bundle) = transaction.ironwood_bundle() {
            self.ironwood.extend(
                bundle
                    .actions()
                    .iter()
                    .map(|action| Digest(action.cmx().to_bytes())),
            );
        }
    }
}

trait TreeNode: Hashable + Clone {
    fn encode_node(&self) -> [u8; 32];
    fn decode_node(bytes: [u8; 32]) -> Option<Self>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SproutNode([u8; 32]);

#[derive(Debug, Clone)]
pub(crate) struct SproutFrontier(Frontier<SproutNode, 29>);
impl SproutFrontier {
    pub(crate) fn root(&self) -> Digest {
        Digest(self.0.root().0)
    }
    pub(crate) fn extend(&mut self, commitments: &[[u8; 32]]) -> Result<()> {
        for cm in commitments {
            if !self.0.append(SproutNode(*cm)) {
                return Err(Error::Full);
            }
        }
        Ok(())
    }
}

impl Canonical for SproutFrontier {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        encode_tree(&self.0, writer)
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        Ok(Self(decode_tree(reader)?))
    }
}

impl Hashable for SproutNode {
    fn empty_leaf() -> Self {
        Self([0; 32])
    }
    fn combine(_level: Level, left: &Self, right: &Self) -> Self {
        let mut block = [0; 64];
        block[..32].copy_from_slice(&left.0);
        block[32..].copy_from_slice(&right.0);
        let mut state = [
            0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
            0x5be0cd19,
        ];
        sha2::compress256(&mut state, &[block.into()]);
        let mut result = [0; 32];
        for (chunk, word) in result.chunks_exact_mut(4).zip(state) {
            chunk.copy_from_slice(&word.to_be_bytes());
        }
        Self(result)
    }
}

impl TreeNode for SproutNode {
    fn encode_node(&self) -> [u8; 32] {
        self.0
    }
    fn decode_node(bytes: [u8; 32]) -> Option<Self> {
        Some(Self(bytes))
    }
}

impl TreeNode for SaplingNode {
    fn encode_node(&self) -> [u8; 32] {
        self.to_bytes()
    }
    fn decode_node(bytes: [u8; 32]) -> Option<Self> {
        Self::from_bytes(bytes).into_option()
    }
}

impl TreeNode for MerkleHashOrchard {
    fn encode_node(&self) -> [u8; 32] {
        self.to_bytes()
    }
    fn decode_node(bytes: [u8; 32]) -> Option<Self> {
        Self::from_bytes(&bytes).into_option()
    }
}

#[derive(Debug, Clone)]
struct CachedNode<H: TreeNode, const PROFILE: u8>(H);
impl<H: TreeNode, const PROFILE: u8> Hashable for CachedNode<H, PROFILE> {
    fn empty_leaf() -> Self {
        Self(H::empty_leaf())
    }
    fn combine(level: Level, left: &Self, right: &Self) -> Self {
        Self(H::combine(level, &left.0, &right.0))
    }
    fn empty_root(level: Level) -> Self {
        let values = match PROFILE {
            0 => &empty::SAPLING,
            1 => &empty::ORCHARD,
            _ => panic!("unsupported tree profile"),
        };
        match H::decode_node(values[usize::from(u8::from(level))]) {
            Some(node) => Self(node),
            None => panic!("invalid pinned empty tree node"),
        }
    }
}
impl<H: TreeNode, const PROFILE: u8> TreeNode for CachedNode<H, PROFILE> {
    fn encode_node(&self) -> [u8; 32] {
        self.0.encode_node()
    }
    fn decode_node(bytes: [u8; 32]) -> Option<Self> {
        H::decode_node(bytes).map(Self)
    }
}
type SaplingHash = CachedNode<SaplingNode, 0>;
type OrchardHash = CachedNode<MerkleHashOrchard, 1>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommitmentTreeSizes {
    pub sprout: u64,
    pub sapling: u64,
    pub orchard: u64,
    pub ironwood: u64,
}

#[derive(Debug, Clone)]
pub struct CandidateTrees {
    sprout: Frontier<SproutNode, 29>,
    sapling: Frontier<SaplingHash, 32>,
    orchard: Frontier<OrchardHash, 32>,
    ironwood: Frontier<OrchardHash, 32>,
    cached_roots: [OnceLock<Digest>; 4],
}

impl Default for CandidateTrees {
    fn default() -> Self {
        Self {
            sprout: Frontier::empty(),
            sapling: Frontier::empty(),
            orchard: Frontier::empty(),
            ironwood: Frontier::empty(),
            cached_roots: Default::default(),
        }
    }
}

impl CandidateTrees {
    pub(crate) fn sprout_frontier(&self) -> SproutFrontier {
        SproutFrontier(self.sprout.clone())
    }
    pub fn roots(&self) -> CommitmentTreeRoots {
        CommitmentTreeRoots {
            sapling: *self.cached_roots[1]
                .get_or_init(|| Digest(self.sapling.root().encode_node())),
            orchard: *self.cached_roots[2]
                .get_or_init(|| Digest(self.orchard.root().encode_node())),
            ironwood: *self.cached_roots[3]
                .get_or_init(|| Digest(self.ironwood.root().encode_node())),
        }
    }
    pub fn sprout_root(&self) -> Digest {
        *self.cached_roots[0].get_or_init(|| Digest(self.sprout.root().0))
    }
    pub fn sizes(&self) -> CommitmentTreeSizes {
        CommitmentTreeSizes {
            sprout: self.sprout.tree_size(),
            sapling: self.sapling.tree_size(),
            orchard: self.orchard.tree_size(),
            ironwood: self.ironwood.tree_size(),
        }
    }
    pub fn commitment(&self) -> Result<Digest> {
        Ok(hash(b"candidate-note-trees", &[&self.to_bytes()?]))
    }
    pub fn append(&mut self, updates: &CommitmentUpdates) -> Result<()> {
        let counts = [
            updates.sprout.len(),
            updates.sapling.len(),
            updates.orchard.len(),
            updates.ironwood.len(),
        ];
        let count = counts
            .into_iter()
            .try_fold(0_usize, |sum, length| sum.checked_add(length))
            .ok_or(Error::Limit)?;
        if count > 100_000 {
            return Err(Error::Limit);
        }
        let mut candidate = self.clone();
        for (cache, count) in candidate.cached_roots.iter_mut().zip(counts) {
            if count != 0 {
                cache.take();
            }
        }
        append(&mut candidate.sprout, &updates.sprout)?;
        append(&mut candidate.sapling, &updates.sapling)?;
        append(&mut candidate.orchard, &updates.orchard)?;
        append(&mut candidate.ironwood, &updates.ironwood)?;
        *self = candidate;
        Ok(())
    }
}

fn append<H: TreeNode, const DEPTH: u8>(
    tree: &mut Frontier<H, DEPTH>,
    updates: &[Digest],
) -> Result<()> {
    for value in updates {
        let node = H::decode_node(value.0).ok_or(Error::Commitment)?;
        if !tree.append(node) {
            return Err(Error::Full);
        }
    }
    Ok(())
}

fn encode_tree<H: TreeNode, const DEPTH: u8>(
    tree: &Frontier<H, DEPTH>,
    writer: &mut Writer,
) -> zutti_protocol::Result<()> {
    writer.u64(tree.tree_size());
    if let Some(frontier) = tree.value() {
        writer.fixed(&frontier.leaf().encode_node());
        writer.u8(u8::try_from(frontier.ommers().len()).map_err(|_| CodecError::Length)?);
        for node in frontier.ommers() {
            writer.fixed(&node.encode_node());
        }
    }
    Ok(())
}

fn decode_tree<H: TreeNode, const DEPTH: u8>(
    reader: &mut Reader<'_>,
) -> zutti_protocol::Result<Frontier<H, DEPTH>> {
    let count = reader.u64()?;
    if count > 1_u64 << DEPTH {
        return Err(CodecError::Length);
    }
    if count == 0 {
        return Ok(Frontier::empty());
    }
    let leaf = H::decode_node(reader.fixed()?).ok_or(CodecError::Invalid("tree node"))?;
    let ommer_count = reader.u8()?;
    if u32::from(ommer_count) != (count - 1).count_ones() {
        return Err(CodecError::Length);
    }
    let mut ommers = Vec::with_capacity(usize::from(ommer_count));
    for _ in 0..ommer_count {
        ommers.push(H::decode_node(reader.fixed()?).ok_or(CodecError::Invalid("tree node"))?);
    }
    Frontier::from_parts(Position::from(count - 1), leaf, ommers)
        .map_err(|_| CodecError::Invalid("tree frontier"))
}

impl Canonical for CandidateTrees {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        writer.fixed(b"ZUTTITRE");
        writer.u16(1);
        encode_tree(&self.sprout, writer)?;
        encode_tree(&self.sapling, writer)?;
        encode_tree(&self.orchard, writer)?;
        encode_tree(&self.ironwood, writer)
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTITRE" || reader.u16()? != 1 {
            return Err(CodecError::Version);
        }
        Ok(Self {
            sprout: decode_tree(reader)?,
            sapling: decode_tree(reader)?,
            orchard: decode_tree(reader)?,
            ironwood: decode_tree(reader)?,
            cached_roots: Default::default(),
        })
    }
}

#[cfg(test)]
mod tests;
