#![allow(clippy::unwrap_used)]
use std::process::Command;
use zutti_protocol::{Digest, Secret32};
use zutti_wallet::backup;

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
