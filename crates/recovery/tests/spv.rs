#![cfg(feature = "spv")]
#![allow(clippy::unwrap_used)]
#[path = "../../../tests/support/spv_pool.rs"]
mod fixture;
#[path = "../../../tests/support/payment.rs"]
mod payment;
#[path = "../../../tests/support/mod.rs"]
mod support;
use std::{path::Path, process::Command};
use zutti_protocol::{Digest, Secret32, encoding::Canonical};
use zutti_wallet::{
    backup,
    spv_pool::{SpvAction, SpvPoolWitness, recovery::SpvSettlementEvidence},
};

fn invoke(arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_zutti-recovery"))
        .args(arguments)
        .env("PATH", "")
        .output()
        .unwrap()
}
fn encrypted(path: &Path, domain: Digest, payload: &[u8], key: &Secret32) {
    backup::persist_new(path, &backup::seal(domain, payload, key).unwrap()).unwrap();
}
#[test]
fn spv_cli_recovers_both_fixed_outcomes_without_original_funding_service() {
    for paid in [true, false] {
        let funding = fixture::reservation(support::digest(52), support::digest(51));
        let expected = fixture::terminal(&funding, paid);
        let SpvAction::Settle { path, window, .. } = &expected.action else {
            unreachable!()
        };
        let evidence = SpvSettlementEvidence {
            root: expected.claim.pool.root,
            path: *path.clone(),
            window: *window.clone(),
        };
        let directory = tempfile::tempdir().unwrap();
        let key = support::secret(150);
        let key_file = directory.path().join("key");
        let input = directory.path().join("funding.backup");
        let record = directory.path().join("record.backup");
        let restored = directory.path().join("restored.backup");
        let observation = directory.path().join("observation.backup");
        let output = directory.path().join("settlement.backup");
        let domain = funding.claim.pool.domain;
        backup::persist_new(&key_file, key.expose()).unwrap();
        encrypted(&input, domain, &funding.to_bytes().unwrap(), &key);
        encrypted(&observation, domain, &evidence.to_bytes().unwrap(), &key);
        let result = invoke(&[
            "spv-export",
            "--backup",
            input.to_str().unwrap(),
            "--key-file",
            key_file.to_str().unwrap(),
            "--domain",
            &domain.to_string(),
            "--out",
            record.to_str().unwrap(),
        ]);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let saved = zutti_wallet::spv_pool::recovery::SpvSwapRecord::read(
            &record,
            &key,
            domain,
            zutti_wallet::spv_pool::recovery::SpvSwapRecord::from_reservation(&funding)
                .unwrap()
                .id()
                .unwrap(),
        )
        .unwrap();
        let swap = saved.id().unwrap().to_string();
        let restore = |swap: &str| {
            invoke(&[
                "spv-restore",
                "--backup",
                record.to_str().unwrap(),
                "--key-file",
                key_file.to_str().unwrap(),
                "--domain",
                &domain.to_string(),
                "--swap",
                swap,
                "--out",
                restored.to_str().unwrap(),
            ])
        };
        assert!(!restore(&support::digest(120).to_string()).status.success());
        assert!(!restored.exists());
        assert!(restore(&swap).status.success());
        std::fs::remove_file(input).unwrap();
        std::fs::remove_file(record).unwrap();
        let prepare = || {
            invoke(&[
                "spv-prepare",
                "--backup",
                restored.to_str().unwrap(),
                "--key-file",
                key_file.to_str().unwrap(),
                "--domain",
                &domain.to_string(),
                "--swap",
                &swap,
                "--evidence",
                observation.to_str().unwrap(),
                "--out",
                output.to_str().unwrap(),
            ])
        };
        let result = prepare();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(stdout.contains("proof_generated=false"));
        assert!(stdout.contains("transaction_submitted=false"));
        let archive = backup::read_private(&output, backup::MAX_ARCHIVE).unwrap();
        let witness =
            SpvPoolWitness::from_bytes(&backup::open(domain, &archive, &key).unwrap()).unwrap();
        assert_eq!(witness.evaluate().unwrap(), expected.claim);
        assert!(!prepare().status.success());
        assert_eq!(
            backup::read_private(&output, backup::MAX_ARCHIVE).unwrap(),
            archive
        );
    }
}
#[test]
fn spv_cli_does_not_promote_generic_encrypted_archives_to_settlement_witnesses() {
    let directory = tempfile::tempdir().unwrap();
    let key = support::secret(150);
    let key_file = directory.path().join("key");
    let input = directory.path().join("input");
    let output = directory.path().join("output");
    let domain = support::digest(151);
    backup::persist_new(&key_file, key.expose()).unwrap();
    encrypted(&input, domain, b"unverified private data", &key);
    let result = invoke(&[
        "spv-export",
        "--backup",
        input.to_str().unwrap(),
        "--key-file",
        key_file.to_str().unwrap(),
        "--domain",
        &domain.to_string(),
        "--out",
        output.to_str().unwrap(),
    ]);
    assert!(!result.status.success());
    assert!(!output.exists());
    assert!(!String::from_utf8_lossy(&result.stderr).contains("unverified private data"));
}
