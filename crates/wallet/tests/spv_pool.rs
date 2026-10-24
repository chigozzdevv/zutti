#![cfg(feature = "spv-pool")]
#![allow(clippy::unwrap_used)]
#[path = "../../../tests/support/spv_pool.rs"]
mod fixture;
#[path = "../../../tests/support/payment.rs"]
mod payment;
#[path = "../../../tests/support/mod.rs"]
mod support;
use zutti_protocol::{Digest, encoding::Canonical, pool::PoolClaim};
use zutti_wallet::spv_pool::{SpvAction, SpvPoolClaim, SpvPoolWitness};

fn funding() -> SpvPoolWitness {
    fixture::reservation(support::digest(52), support::digest(51))
}
fn copy(witness: &SpvPoolWitness) -> SpvPoolWitness {
    SpvPoolWitness::from_bytes(&witness.to_bytes().unwrap()).unwrap()
}

#[test]
fn reservation_checks_backing_authority_outputs_and_exact_proof_assumptions() {
    let witness = funding();
    assert_eq!(witness.evaluate().unwrap(), witness.claim);
    assert_eq!(copy(&witness).evaluate().unwrap(), witness.claim);
    let assumptions = witness.assumptions().unwrap();
    assert_eq!(assumptions.len(), 2);
    assert_eq!(assumptions[0].0, witness.claim.binding_program);
    assert_eq!(assumptions[1].0, witness.policy.program());
    let SpvAction::Reserve(reservation) = &witness.action else {
        unreachable!()
    };
    assert_eq!(assumptions[0].1, reservation.binding.to_bytes().unwrap());
    assert_eq!(assumptions[1].1, reservation.anchor.to_bytes().unwrap());
    assert!(PoolClaim::from_bytes(&witness.claim.to_bytes().unwrap()).is_err());
    assert!(SpvPoolClaim::from_bytes(&witness.claim.pool.to_bytes().unwrap()).is_err());
}
#[test]
fn payout_and_refund_use_fixed_notes_and_the_same_terminal_nullifier() {
    let funding = funding();
    let paid = fixture::terminal(&funding, true);
    let unpaid = fixture::terminal(&funding, false);
    paid.evaluate().unwrap();
    unpaid.evaluate().unwrap();
    assert_eq!(paid.claim.pool.nullifiers, unpaid.claim.pool.nullifiers);
    assert_ne!(paid.claim.pool.outputs, unpaid.claim.pool.outputs);
    assert_eq!(paid.assumptions().unwrap().len(), 2);
    assert_eq!(paid.outputs[0].note.amount().units(), 199_000_000);
    assert_eq!(copy(&paid).evaluate().unwrap(), paid.claim);
}
#[test]
fn funding_context_and_signature_mutations_fail() {
    let original = funding();
    for field in 0..7 {
        let mut bad = copy(&original);
        match field {
            0 => bad.claim.expiry -= 1,
            1 => bad.claim.pool.checkpoint = support::digest(110),
            2 => bad.claim.pool.order_tag = support::digest(111),
            3 => bad.claim.binding_program = support::digest(112),
            4 => bad.claim.policy = support::digest(113),
            5 => bad.inputs[0].path.position += 1,
            _ => {
                bad.outputs[0] =
                    fixture::output(bad.claim.pool.domain, &fixture::keys(20), 102_000_000, 90)
            }
        }
        assert!(bad.evaluate().is_err(), "field {field}");
    }
}
#[test]
fn terminal_rejects_window_policy_ancestry_checkpoint_and_output_substitutions() {
    let original = fixture::terminal(&funding(), true);
    for field in 0..7 {
        let mut bad = copy(&original);
        let SpvAction::Settle {
            window,
            path,
            reservation,
        } = &mut bad.action
        else {
            unreachable!()
        };
        match field {
            0 => window.policy = support::digest(110),
            1 => window.prefix = support::digest(111),
            2 => window.checkpoint = support::digest(112),
            3 => window.reservation = support::digest(113),
            4 => window.output = support::digest(114),
            5 => path.position += 1,
            _ => reservation.txid = support::digest(115),
        }
        assert!(bad.evaluate().is_err(), "field {field}");
    }
}
#[test]
fn terminal_cannot_redirect_output_or_create_unbacked_value() {
    let original = fixture::terminal(&funding(), true);
    for amount in [199_000_000, 200_000_000] {
        let mut bad = copy(&original);
        bad.outputs[0] = fixture::output(bad.claim.pool.domain, &fixture::keys(30), amount, 120);
        bad.claim.pool.outputs =
            zutti_protocol::pool::output_digest(&[bad.outputs[0].binding().unwrap()]).unwrap();
        assert!(bad.evaluate().is_err());
    }
}
#[test]
fn truncated_trailing_legacy_and_oversized_witnesses_fail() {
    let bytes = funding().to_bytes().unwrap();
    for length in [0, 8, bytes.len() - 1] {
        assert!(SpvPoolWitness::from_bytes(&bytes[..length]).is_err());
    }
    let mut trailing = bytes;
    trailing.push(0);
    assert!(SpvPoolWitness::from_bytes(&trailing).is_err());
    assert!(SpvPoolWitness::from_bytes(&vec![0; 262_145]).is_err());
    assert!(SpvPoolClaim::from_bytes(&[0; 16]).is_err());
    assert_ne!(Digest::default(), funding().claim.policy);
}

#[test]
fn recovery_reconstructs_both_fixed_outcomes_without_funding_spending_material() {
    use zutti_wallet::spv_pool::recovery::SpvSwapRecord;
    let funding = funding();
    let record = SpvSwapRecord::from_reservation(&funding).unwrap();
    let bytes = record.to_bytes().unwrap();
    let restored = SpvSwapRecord::from_bytes(&bytes).unwrap();
    assert_eq!(record.id().unwrap(), restored.id().unwrap());
    for paid in [true, false] {
        let expected = fixture::terminal(&funding, paid);
        let SpvAction::Settle { path, window, .. } = expected.action else {
            unreachable!()
        };
        let recovered = restored
            .settlement(expected.claim.pool.root, *path, *window)
            .unwrap();
        assert_eq!(recovered.evaluate().unwrap(), expected.claim);
    }
    assert!(!bytes.windows(32).any(|w| w == support::secret(20).expose()));
    let mut broken = bytes;
    broken[10] ^= 1;
    assert!(SpvSwapRecord::from_bytes(&broken).is_err());
}

#[cfg(feature = "native")]
#[test]
fn encrypted_recovery_files_are_authenticated_and_cannot_be_overwritten() {
    use zutti_wallet::spv_pool::recovery::SpvSwapRecord;
    let funding = funding();
    let record = SpvSwapRecord::from_reservation(&funding).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("swap.backup");
    let key = support::secret(140);
    record.save_new(&path, &key).unwrap();
    assert!(record.save_new(&path, &key).is_err());
    let restored = SpvSwapRecord::read(&path, &key, record.domain(), record.id().unwrap()).unwrap();
    assert_eq!(restored.id().unwrap(), record.id().unwrap());
    assert!(
        SpvSwapRecord::read(
            &path,
            &support::secret(141),
            record.domain(),
            record.id().unwrap()
        )
        .is_err()
    );
    assert!(SpvSwapRecord::read(&path, &key, record.domain(), support::digest(142)).is_err());
    assert!(SpvSwapRecord::read(&path, &key, support::digest(143), record.id().unwrap()).is_err());
}

#[cfg(feature = "native")]
#[test]
fn spv_payment_journal_restarts_with_the_exact_prepared_effects() {
    use zutti_wallet::{
        payment::{PaymentState, SpvPaymentJournal},
        spv_pool::recovery::SpvSwapRecord,
    };
    let funding = funding();
    let record = SpvSwapRecord::from_reservation(&funding).unwrap();
    let terms = record.terms().clone();
    let payment = payment::fixture_custom(false, "0.001", 4_465_101, |input| {
        input.domain = terms.domain().clone();
        input.payout = terms.payout();
        input.refund = terms.refund();
    });
    let pczt = payment.pczt.serialize().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("payment");
    let domain = record.domain();
    let swap = record.id().unwrap();
    let journal =
        SpvPaymentJournal::create(&path, support::secret(150), record, pczt.clone()).unwrap();
    assert_eq!(journal.state().unwrap(), PaymentState::Prepared);
    assert_eq!(journal.unsigned_pczt(), pczt);
    journal.begin_signing().unwrap();
    let resumed = SpvPaymentJournal::open(&path, support::secret(150), domain, swap).unwrap();
    assert_eq!(resumed.state().unwrap(), PaymentState::Signing);
    assert_eq!(resumed.txid(), journal.txid());
    assert!(resumed.begin_signing().is_err());
    assert!(SpvPaymentJournal::open(&path, support::secret(151), domain, swap).is_err());
    assert!(
        SpvPaymentJournal::open(&path, support::secret(150), domain, support::digest(152)).is_err()
    );
}

#[test]
fn reauthorized_funding_cannot_create_more_value_than_the_input_note() {
    let mut witness = funding();
    let SpvAction::Reserve(reservation) = &witness.action else {
        unreachable!()
    };
    witness.outputs[0] = fixture::output(
        witness.claim.pool.domain,
        &fixture::keys(20),
        102_000_000,
        90,
    );
    witness.claim.pool.outputs = zutti_protocol::pool::output_digest(&[
        reservation.binding().unwrap(),
        witness.outputs[0].binding().unwrap(),
    ])
    .unwrap();
    witness.inputs[0].authorization = fixture::keys(20)
        .authorize(
            &witness.inputs[0].note,
            witness.claim.journal_digest().unwrap(),
        )
        .unwrap();
    assert!(witness.evaluate().is_err());
}
