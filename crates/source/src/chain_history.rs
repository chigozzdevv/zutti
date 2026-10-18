use crate::{
    auth,
    block::{ParsedBlock, PoolTransactionCounts},
    bootstrap::GenesisBlock,
    work::Target,
};
use primitive_types::U256;
use zcash_history::{NodeData, NodeDataV2, NodeDataV3, V1, V2, V3, Version};
use zcash_protocol::consensus::{
    BlockHeight, BranchId, MAIN_NETWORK, NetworkType, NetworkUpgrade, Parameters, TEST_NETWORK,
};
use zutti_protocol::{
    Digest, Error as CodecError,
    commitment::hash,
    encoding::{Canonical, Reader, Writer},
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("chain-history height, parent, network or branch mismatch")]
    Context,
    #[error("chain-history header commitment mismatch")]
    Commitment,
    #[error("invalid chain-history frontier")]
    Frontier,
    #[error("chain-history arithmetic overflow")]
    Overflow,
    #[error(transparent)]
    Source(#[from] crate::Error),
    #[error(transparent)]
    Encoding(#[from] CodecError),
}

pub type Result<T> = core::result::Result<T, Error>;

#[derive(Debug, Clone, Copy)]
pub struct CommitmentTreeRoots {
    pub sapling: Digest,
    pub orchard: Digest,
    pub ironwood: Digest,
}

#[derive(Debug, Clone)]
enum Node {
    V1(NodeData),
    V2(NodeDataV2),
    V3(NodeDataV3),
}

impl Node {
    fn base(&self) -> &NodeData {
        match self {
            Self::V1(node) => node,
            Self::V2(node) => &node.v1,
            Self::V3(node) => &node.v2.v1,
        }
    }
    fn counts(&self) -> (u64, u64, u64) {
        match self {
            Self::V1(node) => (node.sapling_tx, 0, 0),
            Self::V2(node) => (node.v1.sapling_tx, node.orchard_tx, 0),
            Self::V3(node) => (node.v2.v1.sapling_tx, node.v2.orchard_tx, node.ironwood_tx),
        }
    }
    fn bytes(&self) -> Vec<u8> {
        match self {
            Self::V1(node) => V1::to_bytes(node),
            Self::V2(node) => V2::to_bytes(node),
            Self::V3(node) => V3::to_bytes(node),
        }
    }
    fn digest(&self) -> Digest {
        Digest(match self {
            Self::V1(node) => V1::hash(node),
            Self::V2(node) => V2::hash(node),
            Self::V3(node) => V3::hash(node),
        })
    }
    fn read(version: u8, branch: BranchId, bytes: &[u8]) -> Result<Self> {
        let branch = u32::from(branch);
        let node = match version {
            1 => Self::V1(V1::from_bytes(branch, bytes).map_err(|_| Error::Frontier)?),
            2 => Self::V2(V2::from_bytes(branch, bytes).map_err(|_| Error::Frontier)?),
            3 => Self::V3(V3::from_bytes(branch, bytes).map_err(|_| Error::Frontier)?),
            _ => return Err(Error::Frontier),
        };
        if node.bytes() != bytes {
            return Err(Error::Frontier);
        }
        Ok(node)
    }
    fn combine(left: &Self, right: &Self) -> Result<Self> {
        let l = left.base();
        let r = right.base();
        if l.consensus_branch_id != r.consensus_branch_id
            || l.end_height.checked_add(1) != Some(r.start_height)
            || l.subtree_total_work.overflowing_add(r.subtree_total_work).1
        {
            return Err(Error::Frontier);
        }
        let (ls, lo, li) = left.counts();
        let (rs, ro, ri) = right.counts();
        ls.checked_add(rs).ok_or(Error::Overflow)?;
        lo.checked_add(ro).ok_or(Error::Overflow)?;
        li.checked_add(ri).ok_or(Error::Overflow)?;
        match (left, right) {
            (Self::V1(l), Self::V1(r)) => Ok(Self::V1(V1::combine(l, r))),
            (Self::V2(l), Self::V2(r)) => Ok(Self::V2(V2::combine(l, r))),
            (Self::V3(l), Self::V3(r)) => Ok(Self::V3(V3::combine(l, r))),
            _ => Err(Error::Frontier),
        }
    }
    fn leaves(&self) -> Result<u64> {
        self.base()
            .end_height
            .checked_sub(self.base().start_height)
            .and_then(|n| n.checked_add(1))
            .ok_or(Error::Frontier)
    }
}

#[derive(Debug, Clone, Copy)]
struct Profile {
    branch: BranchId,
    start: u32,
    version: u8,
    heartwood: u32,
}

fn profile_for<P: Parameters>(parameters: &P, height: u32) -> Result<Profile> {
    let branch = BranchId::for_height(parameters, BlockHeight::from_u32(height));
    let (upgrade, version) = match branch {
        BranchId::Sprout | BranchId::Overwinter | BranchId::Sapling | BranchId::Blossom => {
            (None, 0)
        }
        BranchId::Heartwood => (Some(NetworkUpgrade::Heartwood), 1),
        BranchId::Canopy => (Some(NetworkUpgrade::Canopy), 1),
        BranchId::Nu5 => (Some(NetworkUpgrade::Nu5), 2),
        BranchId::Nu6 => (Some(NetworkUpgrade::Nu6), 2),
        BranchId::Nu6_1 => (Some(NetworkUpgrade::Nu6_1), 2),
        BranchId::Nu6_2 => (Some(NetworkUpgrade::Nu6_2), 2),
        BranchId::Nu6_3 => (Some(NetworkUpgrade::Nu6_3), 3),
        BranchId::Nu7 => (Some(NetworkUpgrade::Nu7), 3),
    };
    Ok(Profile {
        branch,
        version,
        start: upgrade
            .map(|nu| parameters.activation_height(nu).ok_or(Error::Context))
            .transpose()?
            .map(u32::from)
            .unwrap_or(0),
        heartwood: parameters
            .activation_height(NetworkUpgrade::Heartwood)
            .map(u32::from)
            .ok_or(Error::Context)?,
    })
}

fn profile(network: NetworkType, height: u32) -> Result<Profile> {
    match network {
        NetworkType::Main => profile_for(&MAIN_NETWORK, height),
        NetworkType::Test => profile_for(&TEST_NETWORK, height),
        NetworkType::Regtest => Err(Error::Context),
    }
}

#[derive(Debug, Clone, Copy)]
struct BlockData {
    height: u32,
    hash: Digest,
    parent: Digest,
    branch: BranchId,
    time: u32,
    bits: u32,
    commitment: Digest,
    auth: Digest,
    counts: PoolTransactionCounts,
}

impl From<&ParsedBlock> for BlockData {
    fn from(block: &ParsedBlock) -> Self {
        Self {
            height: block.claimed_height(),
            hash: block.hash(),
            parent: block.parent(),
            branch: block.branch(),
            time: block.header().time,
            bits: block.header().bits,
            commitment: Digest(block.header().final_sapling_root),
            auth: block.auth_data_root(),
            counts: block.pool_transaction_counts(),
        }
    }
}

fn leaf(block: &BlockData, roots: CommitmentTreeRoots, version: u8) -> Result<Node> {
    let target = U256::from_little_endian(&Target::from_compact(block.bits)?.little_endian());
    let divisor = target.checked_add(U256::one()).ok_or(Error::Overflow)?;
    let counts = block.counts;
    let base = NodeData {
        consensus_branch_id: block.branch.into(),
        subtree_commitment: block.hash.0,
        start_time: block.time,
        end_time: block.time,
        start_target: block.bits,
        end_target: block.bits,
        start_sapling_root: roots.sapling.0,
        end_sapling_root: roots.sapling.0,
        subtree_total_work: (U256::MAX - target) / divisor + U256::one(),
        start_height: block.height.into(),
        end_height: block.height.into(),
        sapling_tx: counts.sapling,
    };
    if version == 1 {
        return Ok(Node::V1(base));
    }
    let v2 = NodeDataV2 {
        v1: base,
        start_orchard_root: roots.orchard.0,
        end_orchard_root: roots.orchard.0,
        orchard_tx: counts.orchard,
    };
    match version {
        2 => Ok(Node::V2(v2)),
        3 => Ok(Node::V3(NodeDataV3 {
            v2,
            start_ironwood_root: roots.ironwood.0,
            end_ironwood_root: roots.ironwood.0,
            ironwood_tx: counts.ironwood,
        })),
        _ => Err(Error::Context),
    }
}

#[derive(Debug, Clone)]
pub struct CandidateHistory {
    network: NetworkType,
    height: u32,
    tip: Digest,
    peaks: Vec<Node>,
}

impl CandidateHistory {
    pub fn from_genesis(genesis: &GenesisBlock) -> Self {
        Self {
            network: genesis.network(),
            height: 0,
            tip: genesis.block().hash(),
            peaks: Vec::new(),
        }
    }
    pub const fn height(&self) -> u32 {
        self.height
    }
    pub const fn network(&self) -> NetworkType {
        self.network
    }
    pub const fn tip(&self) -> Digest {
        self.tip
    }
    pub fn root(&self) -> Result<Option<Digest>> {
        let mut peaks = self.peaks.iter();
        let Some(first) = peaks.next() else {
            return Ok(None);
        };
        let mut root = first.clone();
        for peak in peaks {
            root = Node::combine(&root, peak)?;
        }
        Ok(Some(root.digest()))
    }
    pub fn commitment(&self) -> Result<Digest> {
        Ok(hash(b"candidate-chain-history", &[&self.to_bytes()?]))
    }
    pub fn append(&mut self, block: &ParsedBlock, roots: CommitmentTreeRoots) -> Result<()> {
        self.append_data(&BlockData::from(block), roots)
    }
    fn append_data(&mut self, block: &BlockData, roots: CommitmentTreeRoots) -> Result<()> {
        if self.height.checked_add(1) != Some(block.height) || block.parent != self.tip {
            return Err(Error::Context);
        }
        let next = profile(self.network, block.height)?;
        if next.branch != block.branch {
            return Err(Error::Context);
        }
        let expected = if block.height == next.heartwood {
            Some(Digest::default())
        } else if next.version > 0 {
            let previous = self.root()?.ok_or(Error::Frontier)?;
            Some(if next.version >= 2 {
                auth::block_commitment(previous, block.auth)
            } else {
                previous
            })
        } else {
            None
        };
        if expected.is_some_and(|expected| expected != block.commitment) {
            return Err(Error::Commitment);
        }
        let mut peaks = if block.height == next.start {
            Vec::new()
        } else {
            self.peaks.clone()
        };
        if next.version > 0 {
            let mut node = leaf(block, roots, next.version)?;
            while let Some(peak) = peaks.last() {
                if peak.leaves()? != node.leaves()? {
                    break;
                }
                let left = peaks.pop().ok_or(Error::Frontier)?;
                node = Node::combine(&left, &node)?;
            }
            peaks.push(node);
        }
        let candidate = Self {
            network: self.network,
            height: block.height,
            tip: block.hash,
            peaks,
        };
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    fn validate(&self) -> Result<()> {
        let profile = profile(self.network, self.height)?;
        if self.tip == Digest::default()
            || (self.height == 0 && self.tip != crate::bootstrap::genesis_hash(self.network)?)
        {
            return Err(Error::Context);
        }
        if profile.version == 0 {
            return if self.peaks.is_empty() {
                Ok(())
            } else {
                Err(Error::Frontier)
            };
        }
        let leaves = u64::from(self.height) - u64::from(profile.start) + 1;
        if self.peaks.len() != leaves.count_ones() as usize {
            return Err(Error::Frontier);
        }
        let mut start = u64::from(profile.start);
        let mut peaks = self.peaks.iter();
        for level in (0..32).rev().filter(|level| leaves & (1_u64 << level) != 0) {
            let count = 1_u64 << level;
            let node = peaks.next().ok_or(Error::Frontier)?;
            let base = node.base();
            let expected_version = match node {
                Node::V1(_) => 1,
                Node::V2(_) => 2,
                Node::V3(_) => 3,
            };
            let counts = node.counts();
            if expected_version != profile.version
                || base.consensus_branch_id != u32::from(profile.branch)
                || base.start_height != start
                || node.leaves()? != count
                || base.subtree_total_work.is_zero()
                || [counts.0, counts.1, counts.2]
                    .into_iter()
                    .any(|n| n > count * 100_000)
            {
                return Err(Error::Frontier);
            }
            start += count;
        }
        self.root()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;

impl Canonical for CandidateHistory {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        writer.fixed(b"ZUTTIHST");
        writer.u16(1);
        writer.u8(match self.network {
            NetworkType::Main => 1,
            NetworkType::Test => 2,
            NetworkType::Regtest => return Err(CodecError::Invalid("history network")),
        });
        writer.u32(self.height);
        writer.fixed(&self.tip.0);
        writer.u8(u8::try_from(self.peaks.len()).map_err(|_| CodecError::Length)?);
        for peak in &self.peaks {
            writer.bytes(&peak.bytes())?;
        }
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTIHST" || reader.u16()? != 1 {
            return Err(CodecError::Version);
        }
        let network = match reader.u8()? {
            1 => NetworkType::Main,
            2 => NetworkType::Test,
            _ => return Err(CodecError::Invalid("history network")),
        };
        let height = reader.u32()?;
        let tip = Digest(reader.fixed()?);
        let count = reader.u8()?;
        if count > 32 {
            return Err(CodecError::Length);
        }
        let profile =
            profile(network, height).map_err(|_| CodecError::Invalid("history profile"))?;
        let mut peaks = Vec::new();
        for _ in 0..count {
            peaks.push(
                Node::read(
                    profile.version,
                    profile.branch,
                    reader.bytes(zcash_history::MAX_NODE_DATA_SIZE)?,
                )
                .map_err(|_| CodecError::Invalid("history node"))?,
            );
        }
        let state = Self {
            network,
            height,
            tip,
            peaks,
        };
        state
            .validate()
            .map_err(|_| CodecError::Invalid("history frontier"))?;
        Ok(state)
    }
}
