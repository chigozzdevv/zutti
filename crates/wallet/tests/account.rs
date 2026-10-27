#![allow(clippy::unwrap_used)]
use zutti_wallet::account::WalletAccount;

#[test]
fn account_restore_retains_both_chain_receivers() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("account");
    let account = WalletAccount::create(&directory).unwrap();
    let restored = WalletAccount::open(&directory).unwrap();
    assert!(account.zcash_address().unwrap().starts_with("utest1"));
    assert_eq!(
        account.zcash_address().unwrap(),
        restored.zcash_address().unwrap()
    );
    assert_eq!(
        account.private_receiver().unwrap(),
        restored.private_receiver().unwrap()
    );
    assert_ne!(account.source_receiver(), [0; 43]);
    assert_eq!(format!("{account:?}"), "WalletAccount([REDACTED])");
    assert_eq!(format!("{:?}", account.note_keys()), "NoteKeys([REDACTED])");
    assert!(
        !std::fs::read(directory.join("account.backup"))
            .unwrap()
            .windows(32)
            .any(|window| window == account.source_spending_key().to_bytes())
    );
}

#[test]
fn account_creation_never_overwrites_and_uses_independent_keys() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("first");
    let first = WalletAccount::create(&directory).unwrap();
    let original = std::fs::read(directory.join("account.backup")).unwrap();
    assert!(WalletAccount::create(&directory).is_err());
    assert_eq!(
        original,
        std::fs::read(directory.join("account.backup")).unwrap()
    );
    let second = WalletAccount::create(&root.path().join("second")).unwrap();
    assert_ne!(first.source_receiver(), second.source_receiver());
    assert_ne!(
        first.private_receiver().unwrap(),
        second.private_receiver().unwrap()
    );
}

#[test]
fn account_rejects_changed_key_and_changed_ciphertext() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("account");
    WalletAccount::create(&directory).unwrap();
    let key = directory.join("account.key");
    let original = std::fs::read(&key).unwrap();
    let mut changed = original.clone();
    changed[0] ^= 1;
    std::fs::write(&key, &changed).unwrap();
    assert!(WalletAccount::open(&directory).is_err());
    std::fs::write(&key, original).unwrap();
    let archive = directory.join("account.backup");
    let mut changed = std::fs::read(&archive).unwrap();
    *changed.last_mut().unwrap() ^= 1;
    std::fs::write(&archive, changed).unwrap();
    assert!(WalletAccount::open(&directory).is_err());
}

#[cfg(unix)]
#[test]
fn account_rejects_symlinks_and_public_permissions() {
    use std::{
        fs,
        os::unix::fs::{PermissionsExt, symlink},
    };
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("account");
    WalletAccount::create(&directory).unwrap();
    assert_eq!(
        fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let link = root.path().join("link");
    symlink(&directory, &link).unwrap();
    assert!(WalletAccount::open(&link).is_err());
    let key = directory.join("account.key");
    assert_eq!(
        fs::metadata(&key).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fs::set_permissions(&key, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(WalletAccount::open(&directory).is_err());
    fs::set_permissions(&key, fs::Permissions::from_mode(0o600)).unwrap();
    fs::rename(&key, root.path().join("saved.key")).unwrap();
    symlink(root.path().join("saved.key"), &key).unwrap();
    assert!(WalletAccount::open(&directory).is_err());
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(WalletAccount::open(&directory).is_err());
}
