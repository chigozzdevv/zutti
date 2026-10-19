#![allow(clippy::unwrap_used)]

use super::*;
use crate::block::ParsedBlock;
use std::path::PathBuf;
use zcash_protocol::consensus::MAIN_NETWORK;
use zebra_chain::serialization::ZcashDeserializeInto;

fn bytes(height: u32) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../tests/fixtures/block-{height}.txt"));
    hex::decode(std::fs::read_to_string(path).unwrap().trim()).unwrap()
}

fn all_roots(trees: &CandidateTrees) -> [Digest; 4] {
    let roots = trees.roots();
    [
        trees.sprout_root(),
        roots.sapling,
        roots.orchard,
        roots.ironwood,
    ]
}

#[test]
fn root_cache_is_derived_and_does_not_change_the_frontier_encoding() {
    let mut trees = CandidateTrees::default();
    trees
        .append(&CommitmentUpdates {
            sprout: vec![Digest([1; 32])],
            sapling: vec![Digest([2; 32])],
            orchard: vec![Digest([3; 32])],
            ironwood: vec![Digest([4; 32])],
        })
        .unwrap();
    let bytes = trees.to_bytes().unwrap();
    let roots = all_roots(&trees);
    assert!(trees.cached_roots.iter().all(|root| root.get().is_some()));
    assert_eq!(trees.to_bytes().unwrap(), bytes);
    let restored = CandidateTrees::from_bytes(&bytes).unwrap();
    assert!(
        restored
            .cached_roots
            .iter()
            .all(|root| root.get().is_none())
    );
    assert_eq!(all_roots(&restored), roots);
    assert_eq!(restored.to_bytes().unwrap(), bytes);
    trees.append(&CommitmentUpdates::default()).unwrap();
    assert_eq!(all_roots(&trees), roots);
    assert!(trees.cached_roots.iter().all(|root| root.get().is_some()));
}

#[test]
fn root_cache_invalidates_changed_pools_without_affecting_clones() {
    let original = CandidateTrees::default();
    let before = all_roots(&original);
    for changed in 0..4 {
        let mut trees = original.clone();
        let mut updates = CommitmentUpdates::default();
        match changed {
            0 => updates.sprout.push(Digest([1; 32])),
            1 => updates.sapling.push(Digest([2; 32])),
            2 => updates.orchard.push(Digest([3; 32])),
            3 => updates.ironwood.push(Digest([4; 32])),
            _ => unreachable!(),
        }
        trees.append(&updates).unwrap();
        for (index, root) in trees.cached_roots.iter().enumerate() {
            assert_eq!(root.get().is_none(), index == changed);
        }
        let after = all_roots(&trees);
        let uncached = [
            Digest(trees.sprout.root().0),
            Digest(trees.sapling.root().encode_node()),
            Digest(trees.orchard.root().encode_node()),
            Digest(trees.ironwood.root().encode_node()),
        ];
        assert_eq!(after, uncached);
        assert_ne!(after[changed], before[changed]);
        assert_eq!(all_roots(&original), before);
        let encoded = trees.to_bytes().unwrap();
        assert!(
            trees
                .append(&CommitmentUpdates {
                    sapling: vec![Digest([255; 32])],
                    ..Default::default()
                })
                .is_err()
        );
        assert_eq!(trees.to_bytes().unwrap(), encoded);
        assert_eq!(all_roots(&trees), after);
    }
}

#[test]
fn public_block_commitment_updates_match_zebra_trees() {
    let mut ours = CandidateTrees::default();
    let mut sprout = zebra_chain::sprout::tree::NoteCommitmentTree::default();
    let mut sapling = zebra_chain::sapling::tree::NoteCommitmentTree::default();
    let mut orchard = zebra_chain::orchard::tree::NoteCommitmentTree::default();
    let mut ironwood = zebra_chain::orchard::tree::NoteCommitmentTree::default();
    assert_eq!(ours.sprout_root().0, <[u8; 32]>::from(sprout.root()));
    assert_eq!(ours.roots().sapling.0, <[u8; 32]>::from(sapling.root()));
    assert_eq!(ours.roots().orchard.0, <[u8; 32]>::from(orchard.root()));
    assert_eq!(ours.roots().ironwood.0, <[u8; 32]>::from(ironwood.root()));
    for height in [0, 1, 1_687_106, 1_687_107, 1_687_108] {
        let raw = bytes(height);
        let parsed = ParsedBlock::parse(&raw, &MAIN_NETWORK).unwrap();
        let reference: zebra_chain::block::Block = raw.as_slice().zcash_deserialize_into().unwrap();
        let updates = parsed.commitment_updates();
        assert_eq!(
            updates.sprout.len(),
            reference.sprout_note_commitments().count()
        );
        assert_eq!(
            updates.sapling.len(),
            reference.sapling_note_commitments().count()
        );
        assert_eq!(
            updates.orchard.len(),
            reference.orchard_note_commitments().count()
        );
        assert_eq!(
            updates.ironwood.len(),
            reference.ironwood_note_commitments().count()
        );
        for cm in reference.sprout_note_commitments() {
            sprout.append(cm).unwrap();
        }
        for cm in reference.sapling_note_commitments() {
            sapling.append(cm).unwrap();
        }
        for cm in reference.orchard_note_commitments() {
            orchard.append(cm).unwrap();
        }
        for cm in reference.ironwood_note_commitments() {
            ironwood.append(cm).unwrap();
        }
        ours.append(updates).unwrap();
        assert_eq!(ours.sprout_root().0, <[u8; 32]>::from(sprout.root()));
        assert_eq!(ours.roots().sapling.0, <[u8; 32]>::from(sapling.root()));
        assert_eq!(ours.roots().orchard.0, <[u8; 32]>::from(orchard.root()));
        assert_eq!(ours.roots().ironwood.0, <[u8; 32]>::from(ironwood.root()));
        let restored = CandidateTrees::from_bytes(&ours.to_bytes().unwrap()).unwrap();
        assert_eq!(restored.commitment().unwrap(), ours.commitment().unwrap());
        assert_eq!(restored.sizes(), ours.sizes());
    }
}

#[test]
fn sprout_compression_and_appends_match_zebra() {
    let mut ours = CandidateTrees::default();
    let mut reference = zebra_chain::sprout::tree::NoteCommitmentTree::default();
    for index in 1..=65 {
        let commitment = hash(b"test-sprout-commitment", &[&[index]]);
        ours.append(&CommitmentUpdates {
            sprout: vec![commitment],
            ..Default::default()
        })
        .unwrap();
        reference
            .append(zebra_chain::sprout::NoteCommitment::from(commitment.0))
            .unwrap();
        assert_eq!(ours.sprout_root().0, <[u8; 32]>::from(reference.root()));
    }
}

#[test]
fn orchard_and_ironwood_frontiers_remain_separate() {
    let mut trees = CandidateTrees::default();
    let empty = trees.roots();
    trees
        .append(&CommitmentUpdates {
            orchard: vec![Digest([1; 32])],
            ironwood: vec![Digest([2; 32]), Digest([3; 32])],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(trees.sizes().orchard, 1);
    assert_eq!(trees.sizes().ironwood, 2);
    assert_eq!(trees.roots().sapling, empty.sapling);
    assert_ne!(trees.roots().orchard, trees.roots().ironwood);
    let orchard = trees.roots().orchard;
    trees
        .append(&CommitmentUpdates {
            ironwood: vec![Digest([4; 32])],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(trees.roots().orchard, orchard);
}

#[test]
fn invalid_commitments_and_full_trees_leave_all_frontiers_unchanged() {
    let mut trees = CandidateTrees::default();
    let original = trees.commitment().unwrap();
    for updates in [
        CommitmentUpdates {
            sprout: vec![Digest([1; 32])],
            sapling: vec![Digest([255; 32])],
            ..Default::default()
        },
        CommitmentUpdates {
            orchard: vec![Digest([1; 32])],
            ironwood: vec![Digest([255; 32])],
            ..Default::default()
        },
    ] {
        assert!(trees.append(&updates).is_err());
        assert_eq!(trees.commitment().unwrap(), original);
    }
    trees.sprout = Frontier::from_parts(
        Position::from((1_u64 << 29) - 1),
        SproutNode([0; 32]),
        vec![SproutNode([0; 32]); 29],
    )
    .unwrap();
    let full = trees.commitment().unwrap();
    assert!(matches!(
        trees.append(&CommitmentUpdates {
            sprout: vec![Digest([1; 32])],
            ..Default::default()
        }),
        Err(Error::Full)
    ));
    assert_eq!(trees.commitment().unwrap(), full);
}

#[test]
fn frontier_codec_rejects_wrong_shapes_noncanonical_fields_and_trailing_bytes() {
    let trees = CandidateTrees::default();
    let mut bytes = trees.to_bytes().unwrap();
    assert!(CandidateTrees::from_bytes(&bytes[..bytes.len() - 1]).is_err());
    bytes.push(0);
    assert!(CandidateTrees::from_bytes(&bytes).is_err());
    let mut writer = Writer::default();
    writer.u64(1);
    writer.fixed(&[0; 32]);
    writer.u8(1);
    assert!(decode_tree::<SproutNode, 29>(&mut Reader::new(&writer.finish())).is_err());
    let mut writer = Writer::default();
    writer.u64((1_u64 << 32) + 1);
    assert!(decode_tree::<SaplingNode, 32>(&mut Reader::new(&writer.finish())).is_err());
    let mut writer = Writer::default();
    writer.u64(1);
    writer.fixed(&[255; 32]);
    writer.u8(0);
    assert!(decode_tree::<MerkleHashOrchard, 32>(&mut Reader::new(&writer.finish())).is_err());
}

#[test]
fn pinned_empty_nodes_match_upstream_consensus_at_every_depth() {
    for depth in 0..=32 {
        let level = Level::from(depth);
        assert_eq!(
            SaplingHash::empty_root(level).encode_node(),
            SaplingNode::empty_root(level).to_bytes()
        );
        assert_eq!(
            OrchardHash::empty_root(level).encode_node(),
            MerkleHashOrchard::empty_root(level).to_bytes()
        );
    }
}
