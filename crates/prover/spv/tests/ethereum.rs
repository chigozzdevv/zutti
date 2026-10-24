use risc0_zkvm::{
    Groth16Receipt, Groth16ReceiptVerifierParameters, InnerReceipt, MaybePruned, Receipt,
    ReceiptClaim, sha::Digestible,
};
use zutti_protocol::Digest;
use zutti_spv_prototype::ethereum::{seal, wrap};

fn genuine() -> (Digest, Vec<u8>, Receipt, Vec<u8>) {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/pool-lifecycle.json"
    ))
    .unwrap();
    let operation = &fixture["operations"]["deposit"];
    let bytes = |field: &str| hex::decode(operation[field].as_str().unwrap()).unwrap();
    let image = Digest(bytes("image_id").try_into().unwrap());
    let journal = bytes("journal");
    let seal = bytes("seal");
    let parameters = Groth16ReceiptVerifierParameters::default().digest();
    let receipt = Receipt::new(
        InnerReceipt::Groth16(Groth16Receipt::new(
            seal[4..].to_vec(),
            MaybePruned::Value(ReceiptClaim::ok(image.0, journal.clone())),
            parameters,
        )),
        journal.clone(),
    );
    (image, journal, receipt, seal)
}
#[test]
fn ethereum_export_verifies_the_exact_receipt_and_preserves_the_genuine_seal() {
    let (image, journal, receipt, expected) = genuine();
    assert_eq!(seal(&receipt, image, &journal).unwrap(), expected);
    let (_, wrapped) = wrap(&receipt, image, &journal).unwrap();
    assert_eq!(wrapped, expected);
    let mut changed = journal.clone();
    changed[0] ^= 1;
    assert!(seal(&receipt, image, &changed).is_err());
    let mut changed = image;
    changed.0[0] ^= 1;
    assert!(wrap(&receipt, changed, &journal).is_err());
    let mut changed = receipt;
    let InnerReceipt::Groth16(inner) = &mut changed.inner else {
        unreachable!()
    };
    inner.seal[100] ^= 1;
    assert!(wrap(&changed, image, &journal).is_err());
}
#[test]
fn unwrapped_or_fabricated_evidence_cannot_be_exported_to_ethereum() {
    let (image, journal, _, _) = genuine();
    let fake = Receipt::new(
        InnerReceipt::Fake(risc0_zkvm::FakeReceipt::new(ReceiptClaim::ok(
            image.0,
            journal.clone(),
        ))),
        journal.clone(),
    );
    assert!(seal(&fake, image, &journal).is_err());
    assert!(wrap(&fake, image, &journal).is_err());
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let composite =
        zutti_spv_prototype::read_receipt(&root.join("fixtures/genesis-v2/receipt.bin")).unwrap();
    let claim = zutti_source::spv::HeaderClaim::from_bytes(&composite.journal.bytes);
    use zutti_protocol::encoding::Canonical;
    let claim = claim.unwrap();
    assert!(
        seal(
            &composite,
            claim.policy().program(),
            &composite.journal.bytes
        )
        .is_err()
    );
}
