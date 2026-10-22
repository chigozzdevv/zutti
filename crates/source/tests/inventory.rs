#![allow(clippy::unwrap_used)]

use std::{
    io::Write,
    process::{Command, Stdio},
};
use zcash_protocol::consensus::{MAIN_NETWORK, TEST_NETWORK};
use zutti_protocol::Digest;
use zutti_source::{Error, block::ParsedBlock, inventory::InventoryChain};

fn raw(height: u32) -> Vec<u8> {
    hex::decode(match height {
        0 => include_str!("../../../tests/fixtures/block-0.txt").trim(),
        1 => include_str!("../../../tests/fixtures/block-1.txt").trim(),
        395 => include_str!("../../../tests/fixtures/block-395.txt").trim(),
        396 => include_str!("../../../tests/fixtures/block-396.txt").trim(),
        1_687_106 => include_str!("../../../tests/fixtures/block-1687106.txt").trim(),
        1_687_107 => include_str!("../../../tests/fixtures/block-1687107.txt").trim(),
        1_687_108 => include_str!("../../../tests/fixtures/block-1687108.txt").trim(),
        _ => panic!("unknown fixture"),
    })
    .unwrap()
}

#[test]
fn genesis_coinbase_has_no_transparent_spend_work() {
    let mut chain = InventoryChain::new(0, Digest([0; 32]));
    let first = chain.append(&raw(0), &MAIN_NETWORK).unwrap();
    assert_eq!(first.counts["transactions"], 1);
    assert_eq!(first.counts["non_coinbase_transactions"], 0);
    assert_eq!(first.counts["transparent_inputs"], 0);
    assert_eq!(first.counts["transparent_outputs"], 1);
    assert_eq!(first.versions.get(&1), Some(&1));
    let second = chain.append(&raw(1), &MAIN_NETWORK).unwrap();
    assert_eq!(second.height, 1);
    assert_eq!(second.parent, first.hash);
}

#[test]
fn invalid_ancestry_does_not_advance_the_inventory() {
    let mut chain = InventoryChain::new(0, Digest([0; 32]));
    assert_eq!(chain.append(&raw(1), &MAIN_NETWORK), Err(Error::Height));
    assert!(chain.append(&raw(0), &TEST_NETWORK).is_err());
    let genesis = chain.append(&raw(0), &MAIN_NETWORK).unwrap();
    assert_eq!(chain.append(&raw(0), &MAIN_NETWORK), Err(Error::Height));
    assert!(chain.append(&raw(1), &MAIN_NETWORK).is_ok());
    let mut wrong_parent = InventoryChain::new(1, Digest([7; 32]));
    assert_eq!(
        wrong_parent.append(&raw(1), &MAIN_NETWORK),
        Err(Error::Parent)
    );
    assert_ne!(genesis.hash, Digest([7; 32]));
}

#[test]
fn sprout_and_modern_workloads_are_distinguished() {
    let sprout = ParsedBlock::parse(&raw(395), &MAIN_NETWORK).unwrap();
    let mut chain = InventoryChain::new(395, sprout.parent());
    let a = chain.append(&raw(395), &MAIN_NETWORK).unwrap();
    let b = chain.append(&raw(396), &MAIN_NETWORK).unwrap();
    assert!(a.counts["sprout_phgr_proofs"] + b.counts["sprout_phgr_proofs"] > 0);
    assert_eq!(a.counts["sprout_groth16_proofs"], 0);
    assert_eq!(b.counts["sprout_groth16_proofs"], 0);
    let bytes = raw(1_687_106);
    let modern = ParsedBlock::parse(&bytes, &MAIN_NETWORK).unwrap();
    let mut chain = InventoryChain::new(modern.claimed_height(), modern.parent());
    let mut orchard_actions = 0;
    let mut sapling_outputs = 0;
    for height in 1_687_106..=1_687_108 {
        let bytes = raw(height);
        let row = chain.append(&bytes, &MAIN_NETWORK).unwrap();
        orchard_actions += row.counts["orchard_actions"];
        sapling_outputs += row.counts["sapling_outputs"];
        assert_eq!(row.counts["sprout_phgr_proofs"], 0);
        assert_eq!(
            row.versions.values().sum::<u64>(),
            row.counts["transactions"]
        );
        assert_eq!(row.bytes, bytes.len());
    }
    assert!(orchard_actions > 0);
    assert!(sapling_outputs > 0);
}

#[test]
fn cli_emits_exact_rows_and_rejects_incomplete_frames() {
    let mut framed = Vec::new();
    for height in [0, 1] {
        let block = raw(height);
        framed.extend_from_slice(&(block.len() as u32).to_le_bytes());
        framed.extend_from_slice(&block);
    }
    let execute = |input: &[u8]| {
        let mut child = Command::new(env!("CARGO_BIN_EXE_source-inventory"))
            .args([
                "--network",
                "mainnet",
                "--start-height",
                "0",
                "--parent",
                &"0".repeat(64),
                "--max-blocks",
                "2",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(input).unwrap();
        child.wait_with_output().unwrap()
    };
    let output = execute(&framed);
    assert!(output.status.success());
    let rows: Vec<serde_json::Value> = output
        .stdout
        .split(|b| *b == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).unwrap())
        .collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["height"], 0);
    assert_eq!(rows[1]["height"], 1);
    assert_eq!(rows[0]["consensus_verified"], false);
    assert_eq!(rows[0]["proof_generated"], false);
    assert_eq!(rows[1]["parent_hash"], rows[0]["block_hash"]);
    assert!(!execute(&framed[..framed.len() - 1]).status.success());
    assert!(!execute(&[1, 0]).status.success());
    assert!(!execute(&[0, 0, 0, 0]).status.success());
}
