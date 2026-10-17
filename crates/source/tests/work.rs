#![allow(clippy::unwrap_used)]
use zcash_protocol::consensus::{MAIN_NETWORK, NetworkType};
use zutti_protocol::Digest;
use zutti_source::{
    block::ParsedBlock,
    bootstrap::{GenesisBlock, genesis_hash},
    difficulty::HeaderCandidate,
    work::{Target, verify_work},
};

#[test]
fn header_context_starts_at_genesis_and_checks_the_real_first_block_atomically() {
    let bytes = hex::decode(include_str!("../../../tests/fixtures/block-0.txt").trim()).unwrap();
    let genesis = GenesisBlock::parse(&bytes, NetworkType::Main).unwrap();
    let first = ParsedBlock::parse(
        &hex::decode(include_str!("../../../tests/fixtures/block-1.txt").trim()).unwrap(),
        &MAIN_NETWORK,
    )
    .unwrap();
    let mut headers = HeaderCandidate::from_genesis(&genesis).unwrap();
    let initial_work = headers.cumulative_work().unwrap();
    assert_eq!(
        headers.expected_bits(first.header().time).unwrap(),
        first.header().bits
    );
    headers.append(&first).unwrap();
    assert_eq!(headers.height().unwrap(), 1);
    assert_eq!(headers.tip().unwrap(), first.hash());
    let work = headers.cumulative_work().unwrap();
    assert!(work > initial_work);
    assert!(headers.append(&first).is_err());
    assert_eq!(headers.cumulative_work().unwrap(), work);
    assert_eq!(headers.tip().unwrap(), first.hash());
}

#[test]
fn public_block_vectors_satisfy_equihash_and_their_claimed_target() {
    for text in [
        include_str!("../../../tests/fixtures/block-1.txt"),
        include_str!("../../../tests/fixtures/block-1687106.txt"),
        include_str!("../../../tests/fixtures/block-1687107.txt"),
        include_str!("../../../tests/fixtures/block-1687108.txt"),
    ] {
        let block = ParsedBlock::parse(&hex::decode(text.trim()).unwrap(), &MAIN_NETWORK).unwrap();
        let checked = verify_work(&block, NetworkType::Main).unwrap();
        assert_eq!(checked.block(), block.hash());
        assert!(checked.target().accepts(block.hash()));
    }
}

#[test]
fn modified_nonce_and_solution_fail_work_validation() {
    let original =
        hex::decode(include_str!("../../../tests/fixtures/block-1687106.txt").trim()).unwrap();
    for offset in [108, 140 + 3, 140 + 3 + 1343] {
        let mut bytes = original.clone();
        bytes[offset] ^= 1;
        let block = ParsedBlock::parse(&bytes, &MAIN_NETWORK).unwrap();
        assert!(verify_work(&block, NetworkType::Main).is_err());
    }
}

#[test]
fn genesis_requires_the_exact_network_identity_and_body() {
    let main = hex::decode(include_str!("../../../tests/fixtures/block-0.txt").trim()).unwrap();
    let test =
        hex::decode(include_str!("../../../tests/fixtures/block-test-0.txt").trim()).unwrap();
    for (bytes, network) in [(&main, NetworkType::Main), (&test, NetworkType::Test)] {
        let genesis = GenesisBlock::parse(bytes, network).unwrap();
        assert_eq!(genesis.block().claimed_height(), 0);
        assert_eq!(genesis.block().hash(), genesis_hash(network).unwrap());
        assert_eq!(genesis.network(), network);
    }
    assert!(GenesisBlock::parse(&main, NetworkType::Test).is_err());
    assert!(GenesisBlock::parse(&test, NetworkType::Main).is_err());
    assert!(GenesisBlock::parse(&main, NetworkType::Regtest).is_err());
    let mut altered = main.clone();
    altered[108] ^= 1;
    assert!(GenesisBlock::parse(&altered, NetworkType::Main).is_err());
    let mut altered = main;
    let end = altered.len() - 1;
    altered[end] ^= 1;
    assert!(GenesisBlock::parse(&altered, NetworkType::Main).is_err());
    let first = hex::decode(include_str!("../../../tests/fixtures/block-1.txt").trim()).unwrap();
    let first = ParsedBlock::parse(&first, &MAIN_NETWORK).unwrap();
    assert_eq!(first.parent(), genesis_hash(NetworkType::Main).unwrap());
}

#[test]
fn compact_targets_reject_sign_zero_and_overflow_and_use_little_endian_hashes() {
    for compact in [
        0,
        0x0180_0000,
        0x0100_0001,
        0x0200_0001,
        0x2300_0001,
        0x2200_0100,
        0x2101_0000,
    ] {
        assert!(Target::from_compact(compact).is_err(), "{compact:08x}");
    }
    let target = Target::from_compact(0x0301_2345).unwrap();
    assert_eq!(&target.little_endian()[..4], &[0x45, 0x23, 0x01, 0]);
    assert!(target.accepts(Digest(target.little_endian())));
    let mut larger = target.little_endian();
    larger[0] += 1;
    assert!(!target.accepts(Digest(larger)));
    assert_eq!(
        Target::from_compact(0x0101_0000).unwrap().little_endian()[0],
        1
    );
    assert!(Target::from_compact(0x2200_00ff).is_ok());
    assert!(Target::from_compact(0x2100_ffff).is_ok());
}
