#![cfg(all(feature = "native", feature = "spv-pool"))]

use incrementalmerkletree::frontier::CommitmentTree;
use orchard::{
    keys::{FullViewingKey, SpendingKey},
    tree::MerkleHashOrchard,
};
use zcash_primitives::merkle_tree::write_commitment_tree;
use zcash_protocol::consensus::TEST_NETWORK;
use zutti_protocol::encoding::Canonical;
use zutti_source::block::ParsedBlock;
use zutti_wallet::{
    Error,
    source_notes::{scan_ironwood, scan_ironwood_stream},
};

#[test]
fn streamed_history_matches_buffered_history_and_checks_the_chain() {
    let viewing = FullViewingKey::from(&SpendingKey::from_bytes([7; 32]).unwrap());
    let blocks: Vec<_> = (1..=3)
        .map(|h| zebra_test::vectors::TESTNET_BLOCKS[&h].to_vec())
        .collect();
    let parent = ParsedBlock::parse(&blocks[0], &TEST_NETWORK)
        .unwrap()
        .parent();
    let tree = CommitmentTree::<MerkleHashOrchard, 32>::empty();
    let mut encoded = Vec::new();
    write_commitment_tree(&tree, &mut encoded).unwrap();
    let root = tree.root().to_bytes();
    let buffered = scan_ironwood(&viewing, 0, parent, &encoded, &blocks, root).unwrap();
    let streamed = scan_ironwood_stream(
        &viewing,
        0,
        parent,
        &encoded,
        blocks.iter().map(|block| Ok(block.as_slice())),
        root,
    )
    .unwrap();
    assert_eq!(streamed.to_bytes().unwrap(), buffered.to_bytes().unwrap());
    assert_eq!(streamed.height, 3);
    assert!(
        scan_ironwood_stream(
            &viewing,
            0,
            parent,
            &encoded,
            [Ok(blocks[0].as_slice()), Ok(blocks[2].as_slice())],
            root
        )
        .is_err()
    );
    let mut wrong_root = root;
    wrong_root[0] ^= 1;
    assert!(
        scan_ironwood_stream(
            &viewing,
            0,
            parent,
            &encoded,
            blocks.iter().map(|block| Ok(block.as_slice())),
            wrong_root
        )
        .is_err()
    );
}

#[test]
fn a_failed_stream_stops_before_reading_later_blocks() {
    let viewing = FullViewingKey::from(&SpendingKey::from_bytes([7; 32]).unwrap());
    let first = zebra_test::vectors::TESTNET_BLOCKS[&1];
    let parent = ParsedBlock::parse(first, &TEST_NETWORK).unwrap().parent();
    let tree = CommitmentTree::<MerkleHashOrchard, 32>::empty();
    let mut encoded = Vec::new();
    write_commitment_tree(&tree, &mut encoded).unwrap();
    let reads = std::cell::Cell::new(0);
    let blocks = (0..3).map(|index| {
        reads.set(reads.get() + 1);
        if index == 1 {
            Err(Error::Io(std::io::Error::other("read failed")))
        } else {
            Ok(first)
        }
    });
    assert!(matches!(
        scan_ironwood_stream(
            &viewing,
            0,
            parent,
            &encoded,
            blocks,
            tree.root().to_bytes()
        ),
        Err(Error::Io(_))
    ));
    assert_eq!(reads.get(), 2);
}
