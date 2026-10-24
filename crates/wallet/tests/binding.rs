#![cfg(feature = "spv")]
#![allow(clippy::unwrap_used)]
#[path = "../../../tests/support/payment.rs"]
mod payment;
#[path = "../../../tests/support/mod.rs"]
mod support;

use orchard::keys::SpendAuthorizingKey;
use pczt::{
    Pczt,
    roles::{signer::Signer, verifier::Verifier},
};
use rand::{SeedableRng, rngs::StdRng};
use zcash_primitives::transaction::txid::{TxIdDigester, to_txid};
use zcash_protocol::consensus::NetworkType;
use zutti_protocol::{
    Digest,
    amount::{Amount, Asset},
    domain::Domain,
    encoding::Canonical,
    order::{OrderInput, OrderTerms},
    policy::PaymentWindow,
};
use zutti_source::spv::{Policy, WindowClaim, WindowTerms};
use zutti_wallet::{
    binding::{PaymentBindingClaim, PaymentBindingWitness, payment_profile},
    preparation::PreparationWitness,
    prepare::inspect_ironwood,
};

fn policy() -> Policy {
    Policy::new(Digest([51; 32]), NetworkType::Test, 2, 128, 4_465_200).unwrap()
}
fn configure(input: &mut OrderInput) {
    input.domain = Domain::new(
        input.domain.source_genesis(),
        11155111,
        [2; 20],
        [3; 20],
        payment_profile(),
        policy().id().unwrap(),
        support::digest(6),
    )
    .unwrap();
}
fn fixture() -> payment::Fixture {
    payment::fixture_custom(false, "0.001", 4_465_101, configure)
}
fn make(terms: OrderTerms, bytes: Vec<u8>, selected: Policy) -> PaymentBindingWitness {
    PaymentBindingWitness::new(
        selected,
        PreparationWitness::new(terms, support::secret(21), support::secret(32), bytes).unwrap(),
    )
    .unwrap()
}
fn input(f: &payment::Fixture) -> OrderInput {
    let mut input = support::input();
    input.domain = f.terms.domain().clone();
    input.window = f.terms.window();
    input.receiver = *f.terms.receiver();
    input
}
fn window(f: &payment::Fixture) -> WindowTerms {
    WindowTerms::new(
        f.terms.domain().id().unwrap(),
        f.terms.commitment(&support::secret(21)).unwrap(),
        f.terms.window().anchor_height(),
        f.terms.window().anchor_hash(),
        f.terms.window().expiry(),
        f.terms.payout().commitment(),
        f.terms.refund().commitment(),
    )
    .unwrap()
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
fn actual_ironwood_effects_bind_both_reservations_and_the_exact_window() {
    let f = fixture();
    let bytes = f.pczt.clone().serialize().unwrap();
    let inspected = inspect_ironwood(&bytes, &f.terms, &support::secret(21)).unwrap();
    let preparation = PreparationWitness::new(
        f.terms.clone(),
        support::secret(21),
        support::secret(32),
        bytes.clone(),
    )
    .unwrap()
    .evaluate()
    .unwrap();
    let witness = make(f.terms.clone(), bytes, policy());
    let claim = witness.evaluate().unwrap();
    assert_eq!(claim.domain(), preparation.domain());
    assert_eq!(claim.order(), preparation.order());
    assert_eq!(claim.reservation(), preparation.reservation());
    assert_eq!(claim.policy(), policy().id().unwrap());
    let terms = WindowTerms::new(
        claim.domain(),
        claim.order(),
        f.terms.window().anchor_height(),
        f.terms.window().anchor_hash(),
        f.terms.window().expiry(),
        f.terms.payout().commitment(),
        f.terms.refund().commitment(),
    )
    .unwrap();
    assert_eq!(claim.window(), terms.id().unwrap());
    assert_eq!(
        claim.source_reservation(),
        terms
            .reservation(policy(), inspected.txid(), &support::secret(32))
            .unwrap()
    );
    let journal = claim.to_bytes().unwrap();
    assert!(!journal.windows(32).any(|w| w == inspected.txid().0));
    assert!(!journal.windows(43).any(|w| w == f.terms.receiver()));
    assert_eq!(PaymentBindingClaim::from_bytes(&journal).unwrap(), claim);
    assert_eq!(
        PaymentBindingWitness::from_bytes(&witness.to_bytes().unwrap())
            .unwrap()
            .evaluate()
            .unwrap(),
        claim
    );
    assert_eq!(format!("{witness:?}"), "PaymentBindingWitness([REDACTED])");
}

#[test]
fn changed_recipient_amount_expiry_anchor_and_height_fail() {
    let f = fixture();
    let bytes = f.pczt.clone().serialize().unwrap();
    for field in 0..5 {
        let mut changed = input(&f);
        match field {
            0 => changed.receiver = [9; 43],
            1 => changed.zec = Amount::parse(Asset::Zec, "4").unwrap(),
            2 => {
                changed.window =
                    PaymentWindow::new(0, 4_465_100, support::digest(13), 4_465_120).unwrap()
            }
            3 => {
                changed.window =
                    PaymentWindow::new(0, 4_465_100, support::digest(15), 4_465_121).unwrap()
            }
            _ => {
                changed.window =
                    PaymentWindow::new(0, 4_465_099, support::digest(13), 4_465_121).unwrap()
            }
        }
        assert!(
            make(OrderTerms::new(changed).unwrap(), bytes.clone(), policy())
                .evaluate()
                .is_err(),
            "field {field}"
        );
    }
}

#[test]
fn policy_profile_network_horizon_and_window_size_cannot_be_substituted() {
    let f = fixture();
    let bytes = f.pczt.clone().serialize().unwrap();
    for selected in [
        Policy::new(Digest([52; 32]), NetworkType::Test, 2, 128, 4_465_200).unwrap(),
        Policy::new(Digest([51; 32]), NetworkType::Main, 2, 128, 4_465_200).unwrap(),
    ] {
        assert!(
            make(f.terms.clone(), bytes.clone(), selected)
                .evaluate()
                .is_err()
        );
    }
    let legacy = payment::fixture(false);
    assert!(
        make(legacy.terms, legacy.pczt.serialize().unwrap(), policy())
            .evaluate()
            .is_err()
    );
    for selected in [
        Policy::new(Digest([51; 32]), NetworkType::Test, 2, 10, 4_465_200).unwrap(),
        Policy::new(Digest([51; 32]), NetworkType::Test, 2, 128, 4_465_122).unwrap(),
    ] {
        let mut changed = input(&f);
        changed.domain = Domain::new(
            f.terms.domain().source_genesis(),
            11155111,
            [2; 20],
            [3; 20],
            payment_profile(),
            selected.id().unwrap(),
            support::digest(6),
        )
        .unwrap();
        assert!(
            make(OrderTerms::new(changed).unwrap(), bytes.clone(), selected)
                .evaluate()
                .is_err()
        );
    }
}

#[test]
fn window_link_rejects_a_different_reservation_or_output() {
    let f = fixture();
    let claim = make(
        f.terms.clone(),
        f.pczt.clone().serialize().unwrap(),
        policy(),
    )
    .evaluate()
    .unwrap();
    let terms = window(&f);
    let mut source = WindowClaim {
        policy: claim.policy(),
        prefix: support::digest(80),
        checkpoint: support::digest(81),
        reservation: claim.source_reservation(),
        height: 4_465_123,
        tip: support::digest(82),
        work: [1; 32],
        output: f.terms.payout().commitment(),
    };
    claim.require_window(&terms, &source).unwrap();
    source.output = f.terms.refund().commitment();
    claim.require_window(&terms, &source).unwrap();
    source.output = support::digest(83);
    assert!(claim.require_window(&terms, &source).is_err());
    source.output = f.terms.payout().commitment();
    source.reservation = support::digest(84);
    assert!(claim.require_window(&terms, &source).is_err());
    source.reservation = claim.source_reservation();
    source.policy = support::digest(85);
    assert!(claim.require_window(&terms, &source).is_err());
}

#[test]
fn correctly_reencrypted_payment_cannot_reuse_the_old_window_binding() {
    let original = fixture();
    let updated = payment::fixture_custom(false, "0.001", 4_465_101, |input| {
        configure(input);
        input.window = PaymentWindow::new(0, 4_465_100, support::digest(15), 4_465_121).unwrap();
    });
    let before = make(
        original.terms.clone(),
        original.pczt.clone().serialize().unwrap(),
        policy(),
    )
    .evaluate()
    .unwrap();
    let after = make(
        updated.terms.clone(),
        updated.pczt.clone().serialize().unwrap(),
        policy(),
    )
    .evaluate()
    .unwrap();
    assert_ne!(
        effect_id(original.pczt.clone()),
        effect_id(updated.pczt.clone())
    );
    assert_ne!(before.reservation(), after.reservation());
    assert_ne!(before.source_reservation(), after.source_reservation());
    let source = WindowClaim {
        policy: before.policy(),
        prefix: support::digest(80),
        checkpoint: support::digest(81),
        reservation: before.source_reservation(),
        height: 4_465_123,
        tip: support::digest(82),
        work: [1; 32],
        output: original.terms.payout().commitment(),
    };
    before.require_window(&window(&original), &source).unwrap();
    assert!(after.require_window(&window(&updated), &source).is_err());
    assert!(PaymentBindingClaim::from_bytes(&before.to_bytes().unwrap()[..106]).is_err());
    assert!(WindowClaim::from_bytes(&before.to_bytes().unwrap()).is_err());
}

#[test]
fn anchor_memo_ciphertext_is_effecting_data_and_modified_openings_fail() {
    let f = fixture();
    let mut encrypted = None;
    Verifier::new(f.pczt.clone())
        .with_ironwood::<(), _>(|bundle| {
            for action in bundle.actions() {
                if action
                    .output()
                    .recipient()
                    .map(|r| r.to_raw_address_bytes())
                    .as_ref()
                    == Some(f.terms.receiver())
                {
                    encrypted = Some(action.output().encrypted_note().enc_ciphertext.0);
                }
            }
            Ok(())
        })
        .unwrap()
        .finish();
    let ciphertext = encrypted.unwrap();
    let original = f.pczt.clone().serialize().unwrap();
    let offset = original
        .windows(ciphertext.len())
        .position(|w| w == ciphertext)
        .unwrap();
    let mut changed = original;
    changed[offset + ciphertext.len() - 16 - 512 + 75] ^= 1;
    let parsed = Pczt::parse(&changed).unwrap();
    assert_ne!(effect_id(parsed), effect_id(f.pczt));
    assert!(make(f.terms, changed, policy()).evaluate().is_err());
}

#[test]
fn authorization_preserves_the_payment_window_binding() {
    let f = fixture();
    let before = make(
        f.terms.clone(),
        f.pczt.clone().serialize().unwrap(),
        policy(),
    )
    .evaluate()
    .unwrap();
    let mut signer = Signer::new(f.pczt).unwrap();
    signer
        .sign_ironwood(
            StdRng::from_seed([44; 32]),
            f.spend,
            &SpendAuthorizingKey::from(&f.key),
        )
        .unwrap();
    let after = make(f.terms, signer.finish().serialize().unwrap(), policy())
        .evaluate()
        .unwrap();
    assert_eq!(before, after);
}

#[test]
fn malformed_and_legacy_encodings_fail_closed() {
    let f = fixture();
    let witness = make(f.terms, f.pczt.clone().serialize().unwrap(), policy());
    let bytes = witness.to_bytes().unwrap();
    for length in [0, 8, 32, bytes.len() - 1] {
        assert!(PaymentBindingWitness::from_bytes(&bytes[..length]).is_err());
    }
    let mut changed = bytes;
    changed.push(0);
    assert!(PaymentBindingWitness::from_bytes(&changed).is_err());
    assert!(PaymentBindingClaim::from_bytes(&[0; 234]).is_err());
}
