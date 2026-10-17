#![allow(clippy::unwrap_used)]
use zcash_encoding::CompactSize;
use zcash_primitives::{block::BlockHeader, transaction::Transaction};
use zcash_protocol::consensus::{BranchId, MAIN_NETWORK};
use zebra_chain::{
    block::{
        Block, ChainHistoryBlockTxAuthCommitmentHash, ChainHistoryMmrRootHash, merkle::AuthDataRoot,
    },
    serialization::ZcashDeserialize,
    transaction::AuthDigest,
};
use zutti_protocol::Digest;
use zutti_source::{
    auth::{auth_data_root, block_commitment, check_authorizing_commitment},
    block::ParsedBlock,
};

#[test]
fn authorization_roots_match_zebra_for_historical_block_bodies() {
    for text in [
        include_str!("../../../tests/fixtures/block-0.txt"),
        include_str!("../../../tests/fixtures/block-1.txt"),
        include_str!("../../../tests/fixtures/block-1687106.txt"),
        include_str!("../../../tests/fixtures/block-1687107.txt"),
        include_str!("../../../tests/fixtures/block-1687108.txt"),
    ] {
        let bytes = hex::decode(text.trim()).unwrap();
        let block = ParsedBlock::parse(&bytes, &MAIN_NETWORK).unwrap();
        let reference = Block::zcash_deserialize(bytes.as_slice()).unwrap();
        assert_eq!(
            block.auth_data_root().0,
            <[u8; 32]>::from(reference.auth_data_root())
        );
    }
}

#[test]
fn authorization_tree_zero_padding_and_combined_commitment_match_zebra() {
    assert!(auth_data_root(&[]).is_err());
    let leaves = [Digest([0x42; 32]), Digest([0xaa; 32]), Digest([0x77; 32])];
    let reference = leaves
        .iter()
        .map(|leaf| AuthDigest(leaf.0))
        .collect::<AuthDataRoot>();
    let root = auth_data_root(&leaves).unwrap();
    assert_eq!(root.0, <[u8; 32]>::from(reference));
    let mut padded = leaves.to_vec();
    padded.push(Digest([0; 32]));
    assert_eq!(auth_data_root(&padded).unwrap(), root);
    padded[3] = leaves[2];
    assert_ne!(auth_data_root(&padded).unwrap(), root);
    let history = Digest([0x11; 32]);
    let expected = ChainHistoryBlockTxAuthCommitmentHash::from_commitments(
        &ChainHistoryMmrRootHash::from(history.0),
        &reference,
    );
    assert_eq!(
        block_commitment(history, root).0,
        <[u8; 32]>::from(expected)
    );
}

#[test]
fn authorization_only_mutation_keeps_the_header_and_txids_but_changes_the_auth_root() {
    let mut bytes =
        hex::decode(include_str!("../../../tests/fixtures/block-1687107.txt").trim()).unwrap();
    let block = ParsedBlock::parse(&bytes, &MAIN_NETWORK).unwrap();
    let history = Digest([0x11; 32]);
    bytes[68..100].copy_from_slice(&block_commitment(history, block.auth_data_root()).0);
    let block = ParsedBlock::parse(&bytes, &MAIN_NETWORK).unwrap();
    check_authorizing_commitment(&block, history).unwrap();
    let mut remaining = bytes.as_slice();
    BlockHeader::read(&mut remaining).unwrap();
    CompactSize::read(&mut remaining).unwrap();
    let start = bytes.len() - remaining.len();
    let coinbase = Transaction::read(&mut remaining, BranchId::Nu5).unwrap();
    let end = bytes.len() - remaining.len();
    let script = &coinbase.transparent_bundle().unwrap().vin[0]
        .script_sig()
        .0
        .0;
    assert!(script.len() > 1);
    let offset = bytes[start..end]
        .windows(script.len())
        .position(|value| value == script)
        .unwrap()
        + start;
    bytes[offset + 1] ^= 1;
    let changed = ParsedBlock::parse(&bytes, &MAIN_NETWORK).unwrap();
    assert_eq!(changed.hash(), block.hash());
    assert_eq!(changed.txids(), block.txids());
    assert_ne!(changed.claimed_height(), block.claimed_height());
    assert_ne!(changed.auth_data_root(), block.auth_data_root());
    assert!(check_authorizing_commitment(&changed, history).is_err());
    assert!(check_authorizing_commitment(&block, Digest([0x22; 32])).is_err());
}
