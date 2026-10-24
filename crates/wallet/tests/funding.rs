#![cfg(all(feature = "native", feature = "spv-pool"))]
#![allow(clippy::unwrap_used)]
#[path = "../../../tests/support/spv_pool.rs"]
mod fixture;
#[path = "../../../tests/support/payment.rs"]
mod payment;
#[path = "../../../tests/support/mod.rs"]
mod support;
use zutti_wallet::{
    funding::{FundingConsent, FundingInput, SpvFundingPlan},
    spv_pool::{SpvAction, SpvPoolWitness},
};

fn funding() -> SpvPoolWitness {
    fixture::reservation(support::digest(52), support::digest(51))
}

fn plan(witness: SpvPoolWitness) -> SpvFundingPlan {
    let SpvAction::Reserve(reservation) = witness.action else {
        unreachable!()
    };
    SpvFundingPlan::new(
        witness.policy,
        witness.claim,
        witness
            .inputs
            .into_iter()
            .map(|i| FundingInput {
                note: i.note,
                path: i.path,
            })
            .collect(),
        witness.outputs,
        reservation,
    )
    .unwrap()
}

#[test]
fn local_consent_authorizes_only_the_exact_funding_operation() {
    let original = funding();
    let expected = original.claim.clone();
    let SpvAction::Reserve(reservation) = &original.action else {
        unreachable!()
    };
    let terms = reservation.terms.clone();
    let owner = fixture::keys(20);
    let consent = FundingConsent {
        terms: &terms,
        refund: owner.recipient().unwrap(),
        policy: original.policy,
        binding_program: original.claim.binding_program,
        pool_program: original.claim.pool_program,
    };
    let signed = plan(original).authorize(&consent, &owner).unwrap();
    assert_eq!(signed.evaluate().unwrap(), expected);
}

#[test]
fn altered_terms_refund_owner_and_programs_are_rejected_before_signing() {
    for field in 0..5 {
        let original = funding();
        let SpvAction::Reserve(reservation) = &original.action else {
            unreachable!()
        };
        let terms = if field == 0 {
            payment::fixture_with_cap(false, "0.002").terms
        } else {
            reservation.terms.clone()
        };
        let owner = fixture::keys(20);
        let consent = FundingConsent {
            terms: &terms,
            refund: if field == 1 {
                fixture::keys(30).recipient().unwrap()
            } else {
                owner.recipient().unwrap()
            },
            policy: original.policy,
            binding_program: if field == 2 {
                support::digest(120)
            } else {
                original.claim.binding_program
            },
            pool_program: if field == 3 {
                support::digest(121)
            } else {
                original.claim.pool_program
            },
        };
        let keys = if field == 4 { fixture::keys(30) } else { owner };
        assert!(plan(original).authorize(&consent, &keys).is_err());
    }
}

#[test]
fn funding_change_cannot_be_redirected_even_with_a_consistent_public_digest() {
    let mut original = funding();
    let SpvAction::Reserve(reservation) = &original.action else {
        unreachable!()
    };
    let terms = reservation.terms.clone();
    let owner = fixture::keys(20);
    let consent = FundingConsent {
        terms: &terms,
        refund: owner.recipient().unwrap(),
        policy: original.policy,
        binding_program: original.claim.binding_program,
        pool_program: original.claim.pool_program,
    };
    original.outputs[0] = fixture::output(
        original.claim.pool.domain,
        &fixture::keys(30),
        101_000_000,
        90,
    );
    original.claim.pool.outputs = zutti_protocol::pool::output_digest(&[
        reservation.binding().unwrap(),
        original.outputs[0].binding().unwrap(),
    ])
    .unwrap();
    assert!(plan(original).authorize(&consent, &owner).is_err());
}

#[test]
fn altered_membership_and_value_cannot_obtain_funding_authorization() {
    for field in 0..3 {
        let mut original = funding();
        let SpvAction::Reserve(reservation) = &original.action else {
            unreachable!()
        };
        let terms = reservation.terms.clone();
        let owner = fixture::keys(20);
        let consent = FundingConsent {
            terms: &terms,
            refund: owner.recipient().unwrap(),
            policy: original.policy,
            binding_program: original.claim.binding_program,
            pool_program: original.claim.pool_program,
        };
        match field {
            0 => original.inputs[0].path.position += 1,
            1 => original.claim.pool.nullifiers = support::digest(110),
            _ => original.outputs.clear(),
        }
        assert!(plan(original).authorize(&consent, &owner).is_err());
    }
}
