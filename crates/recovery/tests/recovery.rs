#![allow(clippy::unwrap_used)]
use std::process::Command;
use zutti_protocol::{Digest, Secret32};
use zutti_wallet::backup;

#[path = "../../../tests/support/swap.rs"]
mod support;

use zutti_protocol::{encoding::Canonical, order::Direction};
use zutti_wallet::{
    pool::PoolWitness,
    swap::{SettlementAction, SwapRecord},
};

#[test]
fn archive_inspection_accepts_reordered_flags_without_printing_witnesses() {
    let dir = tempfile::tempdir().unwrap();
    let key = Secret32::from_bytes([5; 32]).unwrap();
    let domain = Digest([6; 32]);
    let payload = b"private transaction id and unpublished witnesses";
    let archive = backup::seal(domain, payload, &key).unwrap();
    let archive_path = dir.path().join("swap.backup");
    let key_path = dir.path().join("recovery.key");
    backup::persist_new(&archive_path, &archive).unwrap();
    backup::persist_new(&key_path, key.expose()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_zutti-recovery"))
        .arg("inspect")
        .arg("--domain")
        .arg(domain.to_string())
        .arg("--key-file")
        .arg(key_path)
        .arg("--backup")
        .arg(archive_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("archive_authenticated=true"));
    assert!(stdout.contains("settlement_authority=false"));
    assert!(!stdout.contains("private transaction"));
}
#[test]
fn claim_and_refund_cannot_fake_success() {
    for command in ["claim", "refund"] {
        let output = Command::new(env!("CARGO_BIN_EXE_zutti-recovery"))
            .arg(command)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("no transaction submitted"));
    }
}

fn invoke(arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_zutti-recovery"))
        .args(arguments)
        .env("PATH", "")
        .output()
        .unwrap()
}

fn encrypted(path: &std::path::Path, domain: Digest, payload: &[u8], key: &Secret32) {
    backup::persist_new(path, &backup::seal(domain, payload, key).unwrap()).unwrap();
}

#[test]
fn independent_cli_restores_and_prepares_both_fixed_outcomes_without_services() {
    for paid in [true, false] {
        let (reserve, evidence, _, _) = support::scenario(paid, Direction::ZecToUsdc, 2);
        let record = SwapRecord::from_reservation(&reserve).unwrap();
        let domain = record.domain().to_string();
        let swap = record.id().unwrap().to_string();
        let directory = tempfile::tempdir().unwrap();
        let reservation = directory.path().join("reservation.backup");
        let key_file = directory.path().join("recovery.key");
        let exported = directory.path().join("swap.backup");
        let restored = directory.path().join("restored.backup");
        let evidence_file = directory.path().join("evidence.backup");
        let output = directory.path().join("settlement.backup");
        let key = support::secret(111);
        backup::persist_new(&key_file, key.expose()).unwrap();
        encrypted(
            &reservation,
            record.domain(),
            &reserve.to_bytes().unwrap(),
            &key,
        );
        encrypted(
            &evidence_file,
            record.domain(),
            &evidence.to_bytes().unwrap(),
            &key,
        );

        let result = invoke(&[
            "export",
            "--backup",
            reservation.to_str().unwrap(),
            "--key-file",
            key_file.to_str().unwrap(),
            "--domain",
            &domain,
            "--out",
            exported.to_str().unwrap(),
        ]);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(String::from_utf8_lossy(&result.stdout).contains(&format!("swap_id={swap}")));
        let result = invoke(&[
            "restore",
            "--backup",
            exported.to_str().unwrap(),
            "--key-file",
            key_file.to_str().unwrap(),
            "--domain",
            &domain,
            "--swap",
            &swap,
            "--out",
            restored.to_str().unwrap(),
        ]);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_ne!(
            std::fs::read(&exported).unwrap(),
            std::fs::read(&restored).unwrap()
        );
        std::fs::remove_file(reservation).unwrap();
        std::fs::remove_file(exported).unwrap();

        let prepare = |command: &str| {
            invoke(&[
                command,
                "--backup",
                restored.to_str().unwrap(),
                "--key-file",
                key_file.to_str().unwrap(),
                "--domain",
                &domain,
                "--swap",
                &swap,
                "--evidence",
                evidence_file.to_str().unwrap(),
                "--out",
                output.to_str().unwrap(),
            ])
        };
        let wrong = if paid {
            "prepare-refund"
        } else {
            "prepare-claim"
        };
        assert!(!prepare(wrong).status.success());
        assert!(!output.exists());
        let correct = if paid {
            "prepare-claim"
        } else {
            "prepare-refund"
        };
        let result = prepare(correct);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(stdout.contains("witness_prepared=true"));
        assert!(stdout.contains("proof_generated=false"));
        assert!(stdout.contains("transaction_submitted=false"));
        assert!(stdout.contains("settlement_authority=false"));
        let archive = backup::read_private(&output, backup::MAX_ARCHIVE).unwrap();
        let bytes = backup::open(record.domain(), &archive, &key).unwrap();
        let witness = PoolWitness::from_bytes(&bytes).unwrap();
        let action = if paid {
            SettlementAction::Claim
        } else {
            SettlementAction::Refund
        };
        assert_eq!(
            witness.evaluate().unwrap(),
            record.settlement(action, &evidence).unwrap().claim
        );
        let retained = std::fs::read(&output).unwrap();
        assert!(!prepare(correct).status.success());
        assert_eq!(std::fs::read(output).unwrap(), retained);
    }
}

#[test]
fn authenticated_generic_archives_do_not_become_swap_records() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("generic.backup");
    let key_file = directory.path().join("key");
    let output = directory.path().join("restored.backup");
    let key = support::secret(111);
    let domain = Digest([6; 32]);
    backup::persist_new(&key_file, key.expose()).unwrap();
    encrypted(
        &input,
        domain,
        b"private transaction id and unpublished witnesses",
        &key,
    );
    let result = invoke(&[
        "restore",
        "--backup",
        input.to_str().unwrap(),
        "--key-file",
        key_file.to_str().unwrap(),
        "--domain",
        &domain.to_string(),
        "--swap",
        &Digest([7; 32]).to_string(),
        "--out",
        output.to_str().unwrap(),
    ]);
    assert!(!result.status.success());
    assert!(!output.exists());
    assert!(!String::from_utf8_lossy(&result.stderr).contains("private transaction"));
}

#[test]
fn refund_preparation_rejects_missing_and_premature_evidence() {
    let (reserve, evidence, _, _) = support::scenario(false, Direction::UsdcToZec, 8);
    let record = SwapRecord::from_reservation(&reserve).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("swap.backup");
    let key_file = directory.path().join("key");
    let evidence_file = directory.path().join("evidence.backup");
    let output = directory.path().join("refund.backup");
    let key = support::secret(111);
    backup::persist_new(&key_file, key.expose()).unwrap();
    record.save_new(&input, &key).unwrap();
    let arguments = [
        "prepare-refund",
        "--backup",
        input.to_str().unwrap(),
        "--key-file",
        key_file.to_str().unwrap(),
        "--domain",
        &record.domain().to_string(),
        "--swap",
        &record.id().unwrap().to_string(),
        "--evidence",
        evidence_file.to_str().unwrap(),
        "--out",
        output.to_str().unwrap(),
    ];
    assert!(!invoke(&arguments).status.success());
    assert!(!output.exists());
    encrypted(
        &evidence_file,
        record.domain(),
        &evidence.to_bytes().unwrap(),
        &key,
    );
    assert!(!invoke(&arguments).status.success());
    assert!(!output.exists());
}

#[test]
fn restore_rejects_another_valid_swap_without_writing_output() {
    let (reserve, _, _, _) = support::scenario(true, Direction::ZecToUsdc, 2);
    let record = SwapRecord::from_reservation(&reserve).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("swap.backup");
    let key_file = directory.path().join("key");
    let output = directory.path().join("restored.backup");
    let key = support::secret(111);
    backup::persist_new(&key_file, key.expose()).unwrap();
    record.save_new(&input, &key).unwrap();
    let result = invoke(&[
        "restore",
        "--backup",
        input.to_str().unwrap(),
        "--key-file",
        key_file.to_str().unwrap(),
        "--domain",
        &record.domain().to_string(),
        "--swap",
        &Digest([114; 32]).to_string(),
        "--out",
        output.to_str().unwrap(),
    ]);
    assert!(!result.status.success());
    assert!(!output.exists());
}
