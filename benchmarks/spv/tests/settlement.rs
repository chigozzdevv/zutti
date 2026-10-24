#[path = "../../../tests/support/spv_pool.rs"]
mod fixture;
#[path = "../../../tests/support/payment.rs"]
mod payment;
#[path = "../../../tests/support/mod.rs"]
mod support;
use risc0_zkvm::{FakeReceipt, InnerReceipt, Receipt, ReceiptClaim};
use zutti_spv_prototype::settlement::verify_dependencies;

#[test]
fn conditional_proving_rejects_missing_and_fabricated_dependency_receipts() {
    let witness = fixture::reservation(support::digest(52), support::digest(51));
    assert!(verify_dependencies(&witness, &[]).is_err());
    let fake: Vec<_> = witness
        .assumptions()
        .unwrap()
        .into_iter()
        .map(|(image, journal)| {
            Receipt::new(
                InnerReceipt::Fake(FakeReceipt::new(ReceiptClaim::ok(image.0, journal.clone()))),
                journal,
            )
        })
        .collect();
    assert!(verify_dependencies(&witness, &fake).is_err());
    let terminal = fixture::terminal(&witness, true);
    assert!(verify_dependencies(&terminal, &fake).is_err());
}
