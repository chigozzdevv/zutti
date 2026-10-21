#![cfg(feature = "native")]
#![allow(clippy::unwrap_used)]
#[path = "../../../tests/support/payment.rs"]
mod payment_fixture;
use payment_fixture as payment;
#[path = "../../../tests/support/payment_swap.rs"]
mod payment_swap;
#[path = "../../../tests/support/mod.rs"]
mod support;
use orchard::keys::SpendAuthorizingKey;
use pczt::roles::signer::{Signer, SpendAuthSignature, extract_orchard_spend_auth_signatures};
use rand::{SeedableRng, rngs::StdRng};
use zutti_wallet::{
    Error,
    payment::{PaymentJournal, PaymentState},
};

fn signatures(fixture: &payment::Fixture) -> Vec<SpendAuthSignature> {
    let mut signer = Signer::new(fixture.pczt.clone()).unwrap();
    signer
        .sign_ironwood(
            StdRng::from_seed([44; 32]),
            fixture.spend,
            &SpendAuthorizingKey::from(&fixture.key),
        )
        .unwrap();
    extract_orchard_spend_auth_signatures(&signer.finish())
        .into_iter()
        .filter(|s| s.action_index() == fixture.spend)
        .collect()
}
#[test]
fn restart_retains_fixed_payment_and_does_not_repeat_signing() {
    let (fixture, record) = payment_swap::fixture();
    let domain = record.domain();
    let swap = record.id().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("payment");
    let journal = PaymentJournal::create(
        &path,
        support::secret(200),
        record,
        fixture.pczt.clone().serialize().unwrap(),
    )
    .unwrap();
    assert_eq!(journal.state().unwrap(), PaymentState::Prepared);
    assert!(journal.record_signatures(&signatures(&fixture)).is_err());
    journal.begin_signing().unwrap();
    assert!(journal.begin_signing().is_err());
    drop(journal);
    let journal = PaymentJournal::open(&path, support::secret(200), domain, swap).unwrap();
    assert_eq!(journal.state().unwrap(), PaymentState::Signing);
    assert!(journal.begin_signing().is_err());
    journal.record_signatures(&signatures(&fixture)).unwrap();
    let txid = journal.txid();
    assert_eq!(journal.state().unwrap(), PaymentState::Signed);
    drop(journal);
    let journal = PaymentJournal::open(&path, support::secret(200), domain, swap).unwrap();
    assert_eq!(journal.state().unwrap(), PaymentState::Signed);
    assert_eq!(journal.txid(), txid);
    assert_eq!(
        journal.unsigned_pczt(),
        fixture.pczt.clone().serialize().unwrap()
    );
    assert!(journal.record_signatures(&signatures(&fixture)).is_err());
    assert!(matches!(journal.finalize(), Err(Error::Transaction)));
    assert!(
        journal
            .finalize_with_proof(payment::fixture(true).pczt.serialize().unwrap())
            .is_err()
    );
    assert!(
        journal
            .finalize_with_proof(fixture.pczt.clone().serialize().unwrap())
            .is_err()
    );
    let mut called = false;
    assert!(
        journal
            .submit(|_| {
                called = true;
                Ok(txid)
            })
            .is_err()
    );
    assert!(!called);
    assert!(!path.join("transaction.backup").exists());
    assert!(PaymentJournal::open(&path, support::secret(201), domain, swap).is_err());
    assert!(PaymentJournal::open(&path, support::secret(200), support::digest(201), swap).is_err());
    assert!(
        PaymentJournal::open(&path, support::secret(200), domain, support::digest(201)).is_err()
    );
}
#[test]
fn wrong_duplicate_missing_and_foreign_pool_signatures_are_rejected() {
    let (fixture, record) = payment_swap::fixture();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("payment");
    let journal = PaymentJournal::create(
        &path,
        support::secret(200),
        record,
        fixture.pczt.clone().serialize().unwrap(),
    )
    .unwrap();
    journal.begin_signing().unwrap();
    let signatures = signatures(&fixture);
    assert!(journal.record_signatures(&[]).is_err());
    assert!(
        journal
            .record_signatures(&[signatures[0].clone(), signatures[0].clone()])
            .is_err()
    );
    for signature in [
        SpendAuthSignature::from_parts(orchard::ValuePool::Ironwood, fixture.spend, [0; 64]),
        SpendAuthSignature::from_parts(
            orchard::ValuePool::Orchard,
            fixture.spend,
            *signatures[0].signature(),
        ),
        SpendAuthSignature::from_parts(
            orchard::ValuePool::Ironwood,
            usize::MAX,
            *signatures[0].signature(),
        ),
    ] {
        assert!(journal.record_signatures(&[signature]).is_err());
    }
    assert_eq!(journal.state().unwrap(), PaymentState::Signing);
    journal.record_signatures(&signatures).unwrap();
}
#[test]
fn journal_rejects_another_payment_unsafe_paths_and_corrupt_stages() {
    let (fixture, record) = payment_swap::fixture();
    let domain = record.domain();
    let swap = record.id().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("payment");
    assert!(
        PaymentJournal::create(
            &path,
            support::secret(200),
            record,
            payment::fixture(true).pczt.serialize().unwrap()
        )
        .is_err()
    );
    assert!(!path.exists());
    let (_, record) = payment_swap::fixture();
    let journal = PaymentJournal::create(
        &path,
        support::secret(200),
        record,
        fixture.pczt.clone().serialize().unwrap(),
    )
    .unwrap();
    journal.begin_signing().unwrap();
    let marker = std::fs::read(path.join("signing.backup")).unwrap();
    std::fs::write(path.join("signed.backup"), &marker).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            path.join("signed.backup"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
    }
    assert!(journal.state().is_err());
    assert!(PaymentJournal::open(&path, support::secret(200), domain, swap).is_err());
    std::fs::remove_file(path.join("signed.backup")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let link = directory.path().join("link");
        symlink(&path, &link).unwrap();
        assert!(PaymentJournal::open(&link, support::secret(200), domain, swap).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(journal.begin_signing().is_err());
    }
}
