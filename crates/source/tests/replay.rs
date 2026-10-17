#![cfg(feature = "cli")]
#![allow(clippy::unwrap_used)]

use std::{
    path::PathBuf,
    process::{Command, Output},
};

fn replay(block: u32, index_start: u32) -> Output {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    Command::new(env!("CARGO_BIN_EXE_zutti-source"))
        .args([
            "replay",
            "--network",
            "mainnet",
            "--format",
            "hex",
            "--rules",
            &"01".repeat(32),
            "--index-start",
            &index_start.to_string(),
            "--genesis",
        ])
        .arg(directory.join("block-0.txt"))
        .arg("--block")
        .arg(directory.join(format!("block-{block}.txt")))
        .output()
        .unwrap()
}

#[test]
fn actual_genesis_prefix_replay_reports_candidate_state_without_settlement_authority() {
    let result = replay(1, 0);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(output["candidate_height"], 1);
    assert_eq!(output["processed_blocks"], 2);
    assert_eq!(output["index_ready"], true);
    assert_eq!(output["index_height"], 1);
    assert_eq!(output["consensus_verified"], false);
    assert_eq!(output["settlement_authority"], false);
}

#[test]
fn replay_rejects_a_gap_and_preserves_unavailable_index_status() {
    let gap = replay(1_687_106, 0);
    assert!(!gap.status.success());
    assert!(gap.stdout.is_empty());
    let pending = replay(1, 100);
    assert!(pending.status.success());
    let output: serde_json::Value = serde_json::from_slice(&pending.stdout).unwrap();
    assert_eq!(output["index_ready"], false);
    assert_eq!(output["index_height"], serde_json::Value::Null);
    assert_eq!(output["index_root"], serde_json::Value::Null);
}
