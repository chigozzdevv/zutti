#![allow(clippy::unwrap_used)]
#[path = "../../../tests/support/payment.rs"]
mod payment;
#[path = "../../../tests/support/mod.rs"]
mod support;
use orchard::keys::SpendAuthorizingKey;
use payment::{fixture, fixture_with_cap};
use pczt::{Pczt, roles::signer::Signer};
use zcash_primitives::transaction::txid::{TxIdDigester, to_txid};
use zutti_protocol::{Digest, order::OrderTerms};
use zutti_wallet::{
    Error,
    prepare::{authorize, inspect_ironwood},
};

#[test]
fn prepared_source_fee_cannot_exceed_the_agreed_cap() {
    let fixture = fixture_with_cap(false, "0.00000001");
    let bytes = fixture.pczt.serialize().unwrap();
    assert!(matches!(
        inspect_ironwood(&bytes, &fixture.terms, &fixture.randomness),
        Err(Error::Payment)
    ));
}
fn effect_id(pczt: Pczt) -> Digest {
    let effects = pczt.into_effects().unwrap();
    Digest(
        *to_txid(
            effects.version(),
            effects.consensus_branch_id(),
            &effects.digest(TxIdDigester),
        )
        .as_ref(),
    )
}
#[test]
fn ironwood_preparation_preserves_transaction_id_after_authorization() {
    let fixture = fixture(false);
    let bytes = fixture.pczt.clone().serialize().unwrap();
    let inspected = inspect_ironwood(&bytes, &fixture.terms, &fixture.randomness).unwrap();
    assert_eq!(inspected.txid(), effect_id(fixture.pczt.clone()));
    let mut signer = Signer::new(fixture.pczt).unwrap();
    signer
        .sign_ironwood(fixture.spend, &SpendAuthorizingKey::from(&fixture.key))
        .unwrap();
    let signed = signer.finish();
    assert_eq!(inspected.txid(), effect_id(signed.clone()));
    assert!(matches!(
        inspect_ironwood(
            &signed.serialize().unwrap(),
            &fixture.terms,
            &fixture.randomness
        ),
        Err(Error::AlreadyAuthorized)
    ));
    assert!(matches!(
        authorize(&inspected),
        Err(Error::AuthorizationUnavailable)
    ));
    assert!(!format!("{inspected:?}").contains(&inspected.txid().to_string()));
}
#[test]
fn wrong_encrypted_memo_and_modified_terms_are_rejected() {
    let wrong = fixture(true);
    assert!(
        inspect_ironwood(
            &wrong.pczt.serialize().unwrap(),
            &wrong.terms,
            &wrong.randomness
        )
        .is_err()
    );
    let fixture = fixture(false);
    let bytes = fixture.pczt.serialize().unwrap();
    assert!(inspect_ironwood(&bytes, &fixture.terms, &support::secret(22)).is_err());
    let mut changed = support::input();
    changed.window = fixture.terms.window();
    // Validly encoded but different recipient must not reuse the prepared payment.
    let changed = OrderTerms::new(changed).unwrap();
    assert!(inspect_ironwood(&bytes, &changed, &fixture.randomness).is_err());
    assert!(inspect_ironwood(b"not a PCZT", &fixture.terms, &fixture.randomness).is_err());
}
