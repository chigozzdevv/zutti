#![allow(clippy::unwrap_used)]

use super::*;
use std::{path::PathBuf, sync::Arc};
use zcash_history::{Entry, Tree};
use zebra_chain::{
    parameters::Network,
    primitives::zcash_history::{BlockCommitmentTreeRoots, Version as ZebraVersion},
    serialization::ZcashDeserializeInto,
};

fn roots() -> CommitmentTreeRoots {
    CommitmentTreeRoots {
        sapling: Digest([1; 32]),
        orchard: Digest([2; 32]),
        ironwood: Digest([3; 32]),
    }
}

fn bytes(height: u32) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../tests/fixtures/block-{height}.txt"));
    hex::decode(std::fs::read_to_string(path).unwrap().trim()).unwrap()
}

fn data(network: NetworkType, height: u32, parent: Digest, commitment: Digest) -> BlockData {
    let profile = profile(network, height).unwrap();
    BlockData {
        height,
        hash: hash(b"test-history-block", &[&height.to_le_bytes()]),
        parent,
        branch: profile.branch,
        time: 1_700_000_000 + height,
        bits: 0x1f07_ffff,
        commitment,
        auth: Digest([8; 32]),
        counts: PoolTransactionCounts {
            sapling: 2,
            orchard: u64::from(profile.version >= 2),
            ironwood: u64::from(profile.version == 3),
        },
    }
}

fn base_mut(node: &mut Node) -> &mut NodeData {
    match node {
        Node::V1(base) => base,
        Node::V2(base) => &mut base.v1,
        Node::V3(base) => &mut base.v2.v1,
    }
}

fn unverified_frontier(network: NetworkType, height: u32) -> CandidateHistory {
    let profile = profile(network, height).unwrap();
    let mut state = CandidateHistory {
        network,
        height,
        tip: hash(b"test-frontier-tip", &[&height.to_le_bytes()]),
        peaks: Vec::new(),
    };
    if profile.version == 0 {
        return state;
    }
    let count = u64::from(height - profile.start) + 1;
    let mut start = profile.start;
    for level in (0..32).rev().filter(|level| count & (1_u64 << level) != 0) {
        let leaves = 1_u32 << level;
        let mut node = leaf(
            &data(network, start, Digest([9; 32]), Digest::default()),
            roots(),
            profile.version,
        )
        .unwrap();
        let base = base_mut(&mut node);
        base.end_height = u64::from(start) + u64::from(leaves) - 1;
        base.subtree_total_work *= U256::from(leaves);
        state.peaks.push(node);
        start += leaves;
    }
    state.validate().unwrap();
    state
}

fn compare_upstream<V: Version>(nodes: impl IntoIterator<Item = V::NodeData>, expected: &[Digest]) {
    let mut nodes = nodes.into_iter();
    let mut tree = Tree::<V>::new(
        1,
        vec![(0, Entry::new_leaf(nodes.next().unwrap()))],
        Vec::new(),
    );
    assert_eq!(V::hash(tree.root_node().unwrap().data()), expected[0].0);
    for (index, node) in nodes.enumerate() {
        tree.append_leaf(node).unwrap();
        assert_eq!(
            V::hash(tree.root_node().unwrap().data()),
            expected[index + 1].0
        );
    }
}

#[test]
#[allow(clippy::unwrap_used)]
fn all_history_versions_match_upstream_after_every_append() {
    for start in [903_000, 1_046_400, 1_687_104, 3_428_143] {
        let profile = profile(NetworkType::Main, start).unwrap();
        let first = data(NetworkType::Main, start, Digest([9; 32]), Digest::default());
        let node = leaf(&first, roots(), profile.version).unwrap();
        let mut state = CandidateHistory {
            network: NetworkType::Main,
            height: start,
            tip: first.hash,
            peaks: vec![node.clone()],
        };
        let mut nodes = vec![node];
        let mut hashes = vec![state.root().unwrap().unwrap()];
        for height in start + 1..start + 65 {
            let previous = state.root().unwrap().unwrap();
            let commitment = if profile.version >= 2 {
                auth::block_commitment(previous, Digest([8; 32]))
            } else {
                previous
            };
            let block = data(NetworkType::Main, height, state.tip, commitment);
            nodes.push(leaf(&block, roots(), profile.version).unwrap());
            state.append_data(&block, roots()).unwrap();
            hashes.push(state.root().unwrap().unwrap());
            assert_eq!(
                CandidateHistory::from_bytes(&state.to_bytes().unwrap())
                    .unwrap()
                    .commitment()
                    .unwrap(),
                state.commitment().unwrap()
            );
            assert!(state.peaks.len() <= 32);
        }
        match profile.version {
            1 => compare_upstream::<V1>(
                nodes.into_iter().map(|n| match n {
                    Node::V1(n) => n,
                    _ => panic!(),
                }),
                &hashes,
            ),
            2 => compare_upstream::<V2>(
                nodes.into_iter().map(|n| match n {
                    Node::V2(n) => n,
                    _ => panic!(),
                }),
                &hashes,
            ),
            3 => compare_upstream::<V3>(
                nodes.into_iter().map(|n| match n {
                    Node::V3(n) => n,
                    _ => panic!(),
                }),
                &hashes,
            ),
            _ => panic!(),
        }
    }
}

#[test]
#[allow(clippy::unwrap_used)]
fn real_block_leaf_metadata_and_counts_match_zebra() {
    let raw = bytes(1_687_106);
    let parsed = ParsedBlock::parse(&raw, &MAIN_NETWORK).unwrap();
    let block: Arc<zebra_chain::block::Block> =
        Arc::new(raw.as_slice().zcash_deserialize_into().unwrap());
    let sapling = zebra_chain::sapling::tree::Root::try_from(roots().sapling.0).unwrap();
    let orchard = zebra_chain::orchard::tree::Root::try_from(roots().orchard.0).unwrap();
    let ironwood = zebra_chain::orchard::tree::Root::try_from(roots().ironwood.0).unwrap();
    let tree_roots = BlockCommitmentTreeRoots {
        sapling: &sapling,
        orchard: &orchard,
        ironwood: &ironwood,
    };
    let expected =
        <V2 as ZebraVersion>::block_to_history_node(block.clone(), &Network::Mainnet, tree_roots);
    assert_eq!(
        leaf(&BlockData::from(&parsed), roots(), 2).unwrap().bytes(),
        V2::to_bytes(&expected)
    );
    let counts = parsed.pool_transaction_counts();
    assert_eq!(counts.sapling, block.sapling_transactions_count());
    assert_eq!(counts.orchard, block.orchard_transactions_count());
    assert_eq!(counts.ironwood, block.ironwood_transactions_count());
}

#[test]
#[allow(clippy::unwrap_used)]
fn genesis_start_and_real_first_block_are_consecutive_and_atomic() {
    let genesis = GenesisBlock::parse(&bytes(0), NetworkType::Main).unwrap();
    let mut state = CandidateHistory::from_genesis(&genesis);
    let first = ParsedBlock::parse(&bytes(1), &MAIN_NETWORK).unwrap();
    state.append(&first, roots()).unwrap();
    assert_eq!(state.height(), 1);
    assert_eq!(state.root().unwrap(), None);
    assert_eq!(state.tip(), first.hash());
    let committed = state.commitment().unwrap();
    assert!(state.append(&first, roots()).is_err());
    assert_eq!(state.commitment().unwrap(), committed);
}

#[test]
#[allow(clippy::unwrap_used)]
fn upgrade_blocks_check_previous_epoch_before_resetting() {
    for (network, start) in [
        (NetworkType::Main, 903_000),
        (NetworkType::Main, 1_046_400),
        (NetworkType::Main, 1_687_104),
        (NetworkType::Main, 3_428_143),
        (NetworkType::Test, 903_800),
        (NetworkType::Test, 1_028_500),
        (NetworkType::Test, 1_842_420),
        (NetworkType::Test, 4_134_000),
    ] {
        let mut state = unverified_frontier(network, start - 1);
        let next = profile(network, start).unwrap();
        assert_eq!(next.start, start);
        let previous = state.root().unwrap();
        let expected = if start == next.heartwood {
            Digest::default()
        } else if next.version >= 2 {
            auth::block_commitment(previous.unwrap(), Digest([8; 32]))
        } else {
            previous.unwrap()
        };
        let mut block = data(network, start, state.tip, Digest([77; 32]));
        let original = state.commitment().unwrap();
        assert!(state.append_data(&block, roots()).is_err());
        assert_eq!(state.commitment().unwrap(), original);
        block.commitment = expected;
        state.append_data(&block, roots()).unwrap();
        assert_eq!(state.peaks.len(), 1);
        assert_eq!(state.peaks[0].base().start_height, u64::from(start));
        assert_eq!(
            state.peaks[0].base().consensus_branch_id,
            u32::from(next.branch)
        );
    }
}

#[test]
#[allow(clippy::unwrap_used)]
fn malformed_frontiers_and_noncanonical_node_encodings_are_rejected() {
    let state = unverified_frontier(NetworkType::Main, 1_687_110);
    let mut reordered = state.clone();
    reordered.peaks.reverse();
    assert!(CandidateHistory::from_bytes(&reordered.to_bytes().unwrap()).is_err());
    let mut omitted = state.clone();
    omitted.peaks.pop();
    assert!(CandidateHistory::from_bytes(&omitted.to_bytes().unwrap()).is_err());
    let mut overflow = state.clone();
    base_mut(&mut overflow.peaks[0]).subtree_total_work = U256::MAX;
    assert!(CandidateHistory::from_bytes(&overflow.to_bytes().unwrap()).is_err());
    let mut bytes = state.to_bytes().unwrap();
    bytes.push(0);
    assert!(CandidateHistory::from_bytes(&bytes).is_err());
    assert!(CandidateHistory::from_bytes(&[0; 64]).is_err());
    let node = &state.peaks[0];
    let original = node.bytes();
    assert_eq!(original[144], 0xfe);
    let mut noncanonical = original[..144].to_vec();
    noncanonical.push(0xff);
    noncanonical.extend_from_slice(&node.base().start_height.to_le_bytes());
    noncanonical.extend_from_slice(&original[149..]);
    assert!(Node::read(2, BranchId::Nu5, &noncanonical).is_err());
}
