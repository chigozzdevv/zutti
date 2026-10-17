#![allow(clippy::unwrap_used)]
use zutti_protocol::{Digest, Secret32};
use zutti_wallet::backup;
#[test]
fn encrypted_recovery_binds_domain_key_and_every_byte() {
    let key = Secret32::from_bytes([1; 32]).unwrap();
    let domain = Digest([2; 32]);
    let archive = backup::seal(domain, b"private unpublished swap witnesses", &key).unwrap();
    assert_eq!(
        backup::open(domain, &archive, &key).unwrap().as_slice(),
        b"private unpublished swap witnesses"
    );
    assert!(backup::open(Digest([3; 32]), &archive, &key).is_err());
    assert!(backup::open(domain, &archive, &Secret32::from_bytes([4; 32]).unwrap()).is_err());
    for index in 0..archive.len() {
        let mut changed = archive.clone();
        changed[index] ^= 1;
        assert!(
            backup::open(domain, &changed, &key).is_err(),
            "tampered byte {index}"
        );
    }
    for len in 0..archive.len() {
        assert!(backup::open(domain, &archive[..len], &key).is_err());
    }
    let mut trailing = archive;
    trailing.push(0);
    assert!(backup::open(domain, &trailing, &key).is_err());
}
#[test]
fn backup_nonce_is_fresh_and_no_plaintext_is_written() {
    let key = Secret32::from_bytes([1; 32]).unwrap();
    let domain = Digest([2; 32]);
    let first = backup::seal(domain, b"unpublished payment secret", &key).unwrap();
    let second = backup::seal(domain, b"unpublished payment secret", &key).unwrap();
    assert_ne!(first, second);
    assert!(
        !first
            .windows(26)
            .any(|w| w == b"unpublished payment secret")
    );
}
#[test]
fn durable_creation_refuses_overwrite_and_insecure_permissions() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("swap.backup");
    let key = Secret32::from_bytes([1; 32]).unwrap();
    let domain = Digest([2; 32]);
    let archive = backup::seal(domain, b"witnesses", &key).unwrap();
    backup::persist_new(&path, &archive).unwrap();
    assert_eq!(
        backup::read_private(&path, backup::MAX_ARCHIVE)
            .unwrap()
            .as_slice(),
        archive
    );
    assert!(backup::persist_new(&path, b"replacement").is_err());
    assert_eq!(std::fs::read(&path).unwrap(), archive);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let link = directory.path().join("link");
        symlink(&path, &link).unwrap();
        assert!(backup::read_private(&link, backup::MAX_ARCHIVE).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(backup::read_private(&path, backup::MAX_ARCHIVE).is_err());
    }
}
