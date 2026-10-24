#![allow(clippy::unwrap_used)]
use super::*;
use crate::test_support::{fixture, support};
use zutti_protocol::encoding::Canonical;
use zutti_wallet::{
    funding::FundingInput,
    spv_pool::{SpvAction, recovery::SpvSwapRecord},
};

fn setup() -> (SpvFundingPlan, SpvAuthorizationProfile) {
    let funding = fixture::reservation(support::digest(52), support::digest(51));
    let record = SpvSwapRecord::from_reservation(&funding).unwrap();
    let (mut ethereum, _, _) = crate::tests::fixture("deneb");
    ethereum.chain_id = 11155111;
    let profile = SpvAuthorizationProfile {
        domain: record.terms().domain().clone(),
        ethereum,
        pool_code_hash: B256::repeat_byte(5),
        binding_program: support::digest(52),
        pool_program: support::digest(60),
        policy: record.reservation().anchor.policy(),
        minimum_blocks: 3,
        maximum_source_age: 90,
        future_drift: 30,
    };
    let SpvAction::Reserve(reservation) = funding.action else {
        unreachable!()
    };
    let plan = SpvFundingPlan::new(
        funding.policy,
        funding.claim,
        funding
            .inputs
            .into_iter()
            .map(|i| FundingInput {
                note: i.note,
                path: i.path,
            })
            .collect(),
        funding.outputs,
        reservation,
    )
    .unwrap();
    (plan, profile)
}

#[test]
fn request_uses_the_locally_pinned_deployment_and_exact_unsigned_funding() {
    let (plan, swap) = setup();
    let profile = SpvFundingProfile {
        swap: &swap,
        registry: Address::repeat_byte(9),
        registry_code_hash: B256::repeat_byte(10),
    };
    let request = profile.request(&plan, &fixture::keys(20)).unwrap();
    assert_eq!(request.root, B256::from(plan.claim().pool.root.0));
    assert_eq!(
        request.checkpoint,
        B256::from(plan.claim().pool.checkpoint.0)
    );
    assert_eq!(request.order_tag, B256::from(plan.claim().pool.order_tag.0));
    assert_eq!(
        request.nullifiers,
        plan.nullifiers(&fixture::keys(20))
            .unwrap()
            .into_iter()
            .map(|n| B256::from(n.0))
            .collect::<Vec<_>>()
    );
    assert_eq!(request.source, profile.registry);
    assert_eq!(request.expiry, plan.claim().expiry);
}

#[test]
fn changed_profile_and_wrong_inventory_owner_are_rejected() {
    for field in 0..5 {
        let (plan, mut swap) = setup();
        match field {
            0 => swap.ethereum.chain_id = 1,
            1 => swap.binding_program = support::digest(110),
            2 => swap.pool_program = support::digest(111),
            3 => swap.minimum_blocks = 129,
            _ => {}
        }
        let profile = SpvFundingProfile {
            swap: &swap,
            registry: Address::repeat_byte(9),
            registry_code_hash: B256::repeat_byte(10),
        };
        let owner = if field == 4 {
            fixture::keys(30)
        } else {
            fixture::keys(20)
        };
        assert!(profile.request(&plan, &owner).is_err());
    }
}

#[test]
fn source_receipt_verification_uses_genuine_evidence_and_rejects_substitution() {
    use bincode::Options;
    use risc0_zkvm::{FakeReceipt, ReceiptClaim};
    use zutti_source::spv::HeaderClaim;
    let receipt: Receipt = bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .reject_trailing_bytes()
        .deserialize(include_bytes!(
            "../../../../benchmarks/spv/fixtures/genesis-coverage/receipt.bin"
        ))
        .unwrap();
    let claim = HeaderClaim::from_bytes(&receipt.journal.bytes).unwrap();
    let program = claim.policy().program();
    let journal = claim.to_bytes().unwrap();
    verify_receipt(program, &journal, &receipt).unwrap();
    assert!(verify_receipt(support::digest(250), &journal, &receipt).is_err());
    let fake = Receipt::new(
        InnerReceipt::Fake(FakeReceipt::new(ReceiptClaim::ok(
            program.0,
            journal.clone(),
        ))),
        journal.clone(),
    );
    assert!(verify_receipt(program, &journal, &fake).is_err());
}
