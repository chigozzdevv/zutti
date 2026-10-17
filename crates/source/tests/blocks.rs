#![allow(clippy::unwrap_used)]
use zcash_protocol::consensus::MAIN_NETWORK;
use zutti_protocol::Digest;
use zutti_source::{IndexContext, Observation, block::ParsedBlock, state::CandidateChain};

fn raw(height: u32) -> Vec<u8> {
    hex::decode(match height {
        1_687_106 => include_str!("../../../tests/fixtures/block-1687106.txt").trim(),
        1_687_107 => include_str!("../../../tests/fixtures/block-1687107.txt").trim(),
        _ => include_str!("../../../tests/fixtures/block-1.txt").trim(),
    })
    .unwrap()
}
#[test]
fn real_historical_blocks_parse_and_link_with_complete_transaction_lists() {
    let first = ParsedBlock::parse(&raw(1_687_106), &MAIN_NETWORK).unwrap();
    let second = ParsedBlock::parse(&raw(1_687_107), &MAIN_NETWORK).unwrap();
    assert_eq!(first.claimed_height(), 1_687_106);
    assert_eq!(second.parent(), first.hash());
    assert!(first.versions().contains(&5) || second.versions().contains(&5));
    let context =
        IndexContext::new(Digest([1; 32]), Digest([2; 32]), first.claimed_height() - 1).unwrap();
    let mut chain = CandidateChain::new(context, first.parent()).unwrap();
    chain.append(&first).unwrap();
    chain.append(&second).unwrap();
    for txid in first.txids() {
        assert_eq!(
            chain.prove(*txid).verify(chain.snapshot(), *txid).unwrap(),
            Observation::Present(first.claimed_height())
        );
    }
    let before = chain.snapshot();
    assert!(chain.append(&first).is_err());
    assert_eq!(chain.snapshot(), before);
    let early = ParsedBlock::parse(&raw(1), &MAIN_NETWORK).unwrap();
    assert_eq!(early.claimed_height(), 1);
    assert_eq!(early.versions(), &[1]);
}
#[test]
fn truncated_trailing_and_modified_block_frames_fail() {
    let original = raw(1_687_106);
    for length in [0, 32, 100, original.len() - 1] {
        assert!(ParsedBlock::parse(&original[..length], &MAIN_NETWORK).is_err());
    }
    let mut trailing = original.clone();
    trailing.push(0);
    assert!(ParsedBlock::parse(&trailing, &MAIN_NETWORK).is_err());
    let mut bad_root = original;
    bad_root[36] ^= 1;
    assert!(ParsedBlock::parse(&bad_root, &MAIN_NETWORK).is_err());
}

#[test]
fn omitted_transactions_and_count_substitution_fail() {
    use zcash_encoding::CompactSize;
    use zcash_primitives::{block::BlockHeader, transaction::Transaction};
    use zcash_protocol::consensus::BranchId;
    let original = raw(1_687_106);
    let mut remaining = original.as_slice();
    let header = BlockHeader::read(&mut remaining).unwrap();
    let count: usize = CompactSize::read_t(&mut remaining).unwrap();
    assert!(count > 1);
    let coinbase = Transaction::read(&mut remaining, BranchId::Sprout).unwrap();
    let mut omitted = Vec::new();
    header.write(&mut omitted).unwrap();
    CompactSize::write(&mut omitted, 1).unwrap();
    coinbase.write(&mut omitted).unwrap();
    assert!(ParsedBlock::parse(&omitted, &MAIN_NETWORK).is_err());
    let mut bad_count = Vec::new();
    header.write(&mut bad_count).unwrap();
    CompactSize::write(&mut bad_count, 100_001).unwrap();
    assert!(ParsedBlock::parse(&bad_count, &MAIN_NETWORK).is_err());
}

#[test]
fn framing_checks_cannot_be_relabelled_as_consensus_validation() {
    let mut altered = raw(1_687_106);
    altered[108] ^= 1;
    let block = ParsedBlock::parse(&altered, &MAIN_NETWORK).unwrap();
    let context =
        IndexContext::new(Digest([1; 32]), Digest([2; 32]), block.claimed_height() - 1).unwrap();
    let mut chain = CandidateChain::new(context, block.parent()).unwrap();
    chain.append(&block).unwrap();
    assert!(
        zutti_source::consensus::verify_history(chain.snapshot(), b"framing is not consensus")
            .is_err()
    );
}

#[test]
fn source_cli_reports_the_verification_boundary() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/block-1687106.txt");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_zutti-source"))
        .args([
            "inspect",
            "--network",
            "mainnet",
            "--format",
            "hex",
            "--block",
        ])
        .arg(fixture)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let status: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(status["claimed_height"], 1_687_106);
    assert_eq!(status["consensus_verified"], false);
}
