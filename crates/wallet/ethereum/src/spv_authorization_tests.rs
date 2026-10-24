#![allow(clippy::unwrap_used)]
use super::*;
use crate::test_support::{fixture, support};
use risc0_zkvm::{FakeReceipt, ReceiptClaim};

fn record() -> SpvSwapRecord {
    SpvSwapRecord::from_reservation(&fixture::reservation(
        support::digest(52),
        support::digest(51),
    ))
    .unwrap()
}
fn profile(record: &SpvSwapRecord) -> SpvAuthorizationProfile {
    let (mut ethereum, _, _) = crate::tests::fixture("deneb");
    ethereum.chain_id = 11155111;
    SpvAuthorizationProfile {
        domain: record.terms().domain().clone(),
        ethereum,
        pool_code_hash: B256::repeat_byte(5),
        binding_program: support::digest(52),
        pool_program: support::digest(60),
        policy: record.reservation().anchor.policy(),
        minimum_blocks: 3,
        maximum_source_age: 90,
        future_drift: 30,
    }
}
#[test]
fn reservation_request_binds_saved_escrow_order_nullifier_and_deployment() {
    let record = record();
    let profile = profile(&record);
    let request = profile.reservation_request(&record).unwrap();
    assert_eq!(
        request.escrow,
        B256::from(record.reservation().escrow.commitment().unwrap().0)
    );
    assert_eq!(
        request.order_tag,
        B256::from(record.funding().pool.order_tag.0)
    );
    assert_eq!(
        request.nullifier,
        B256::from(record.reservation().escrow.nullifier().unwrap().0)
    );
    assert_eq!(request.pool, Address::from(record.terms().domain().pool()));
}
#[test]
fn changed_deployment_image_policy_and_invalid_limits_fail() {
    let record = record();
    for field in 0..6 {
        let mut profile = profile(&record);
        match field {
            0 => profile.binding_program = support::digest(53),
            1 => profile.ethereum.chain_id = 1,
            2 => profile.pool_code_hash = B256::ZERO,
            3 => profile.minimum_blocks = 0,
            4 => profile.maximum_source_age = 0,
            _ => profile.pool_program = support::digest(61),
        }
        assert!(profile.reservation_request(&record).is_err());
    }
}
#[test]
fn matching_funding_journal_does_not_admit_a_fake_receipt() {
    let record = record();
    let journal = record.funding().to_bytes().unwrap();
    let fake = Receipt::new(
        InnerReceipt::Fake(FakeReceipt::new(ReceiptClaim::ok(
            [60; 32],
            journal.clone(),
        ))),
        journal,
    );
    assert!(verify_funding(support::digest(60), &record, &fake).is_err());
}
#[test]
fn source_time_rejects_staleness_and_future_evidence_at_exact_boundaries() {
    source_time(100, 90, 30, 190).unwrap();
    assert!(source_time(100, 90, 30, 191).is_err());
    source_time(130, 90, 30, 100).unwrap();
    assert!(source_time(131, 90, 30, 100).is_err());
}

#[test]
fn signing_requires_the_local_accepted_terms_and_receiving_key() {
    let record = record();
    let intended = fixture::keys(10).recipient().unwrap();
    verify_consent(
        &record,
        &SpvPaymentConsent {
            terms: record.terms(),
            receiving: intended,
        },
    )
    .unwrap();
    assert!(
        verify_consent(
            &record,
            &SpvPaymentConsent {
                terms: record.terms(),
                receiving: fixture::keys(20).recipient().unwrap()
            }
        )
        .is_err()
    );
    let other = zutti_protocol::order::OrderTerms::new(support::input()).unwrap();
    assert!(
        verify_consent(
            &record,
            &SpvPaymentConsent {
                terms: &other,
                receiving: intended
            }
        )
        .is_err()
    );
}

#[test]
fn ethereum_seals_reconstruct_genuine_receipts_and_reject_substitutions() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/pool-lifecycle.json"
    ))
    .unwrap();
    let deposit = &fixture["operations"]["deposit"];
    let bytes = |field: &str| alloy::hex::decode(deposit[field].as_str().unwrap()).unwrap();
    let program = Digest(bytes("image_id").try_into().unwrap());
    let journal = bytes("journal");
    let seal = bytes("seal");
    let receipt = ethereum_receipt(program, &journal, &seal).unwrap();
    verify_receipt(program, &journal, &receipt).unwrap();
    let mut other = program;
    other.0[0] ^= 1;
    assert!(ethereum_receipt(other, &journal, &seal).is_err());
    let mut changed = journal.clone();
    changed[0] ^= 1;
    assert!(ethereum_receipt(program, &changed, &seal).is_err());
    let mut changed = seal.clone();
    changed[0] ^= 1;
    assert!(ethereum_receipt(program, &journal, &changed).is_err());
    let mut changed = seal.clone();
    changed[100] ^= 1;
    assert!(ethereum_receipt(program, &journal, &changed).is_err());
    assert!(ethereum_receipt(program, &journal, &seal[..259]).is_err());
    assert!(recover_funding(&record(), &seal).is_err());
}

#[test]
fn payment_authorization_requires_the_funding_seal_to_be_retained_and_verified() {
    let record = record();
    assert!(retained_funding(&record).is_err());
    let retained = record.with_funding_seal(vec![0; 260]).unwrap();
    assert!(retained_funding(&retained).is_err());
}
