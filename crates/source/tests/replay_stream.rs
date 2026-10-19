#![cfg(all(feature = "storage", feature = "cli"))]
#![allow(clippy::unwrap_used)]
use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
use zcash_protocol::consensus::NetworkType;
use zutti_protocol::{Digest, encoding::Canonical};
use zutti_source::{proof::SourcePolicy, range::SourceRangeWitness, storage::ReplayStore};

fn policy() -> SourcePolicy {
    SourcePolicy {
        program: Digest([21; 32]),
        registry_program: Digest([22; 32]),
        network: NetworkType::Test,
        index_start: 0,
        confirmations: 3,
    }
}
fn execute(base: &Path, through: u32, input: &[u8]) -> std::process::Output {
    let genesis = base.join("genesis.bin");
    std::fs::write(&genesis, zebra_test::vectors::TESTNET_BLOCKS[&0]).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_source-replay"))
        .arg("--network")
        .arg("testnet")
        .arg("--database")
        .arg(base.join("replay.sqlite"))
        .arg("--genesis")
        .arg(genesis)
        .arg("--source-program")
        .arg(policy().program.to_string())
        .arg("--registry-program")
        .arg(policy().registry_program.to_string())
        .args([
            "--confirmations",
            "3",
            "--through-height",
            &through.to_string(),
            "--batch-size",
            "2",
            "--sample-start",
            "2",
            "--sample-size",
            "2",
            "--samples",
        ])
        .arg(base.join("samples"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}
fn frames(heights: &[u32]) -> Vec<u8> {
    heights
        .iter()
        .flat_map(|h| {
            let raw = &zebra_test::vectors::TESTNET_BLOCKS[h];
            let mut frame = (raw.len() as u32).to_le_bytes().to_vec();
            frame.extend_from_slice(raw);
            frame
        })
        .collect()
}
#[test]
fn stream_samples_resume_and_export_only_conditional_range_evidence() {
    let directory = tempfile::tempdir().unwrap();
    let output = execute(directory.path(), 4, &frames(&[1, 2, 3, 4]));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let ready: serde_json::Value =
        serde_json::from_slice(output.stdout.split(|b| *b == b'\n').next().unwrap()).unwrap();
    assert_eq!(ready["native_height"], 0);
    assert_eq!(ready["genesis_anchored_proof"], false);
    let sample = directory.path().join("samples/2-4");
    let witness =
        SourceRangeWitness::from_bytes(&std::fs::read(sample.join("witness.bin")).unwrap())
            .unwrap();
    assert_eq!(witness.evaluate().unwrap().end.height, 4);
    let output = execute(directory.path(), 5, &frames(&[5]));
    assert!(output.status.success());
    let store = ReplayStore::open(&directory.path().join("replay.sqlite"), policy()).unwrap();
    assert_eq!(store.candidate().height().unwrap(), 5);
    assert_eq!(store.samples().unwrap(), [(2, 4)]);
}
#[test]
fn invalid_and_truncated_stream_batches_do_not_advance_the_frontier() {
    for input in [
        frames(&[1, 3]),
        frames(&[1]),
        u32::MAX.to_le_bytes().to_vec(),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let output = execute(directory.path(), 4, &input);
        assert!(!output.status.success());
        let store = ReplayStore::open(&directory.path().join("replay.sqlite"), policy()).unwrap();
        assert_eq!(store.candidate().height().unwrap(), 0);
        assert!(store.samples().unwrap().is_empty());
    }
}
