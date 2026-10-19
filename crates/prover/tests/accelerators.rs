#![allow(clippy::unwrap_used)]

use bincode::Options;
use risc0_zkvm::{InnerReceipt, Receipt, VerifierContext};
use sha2::{Digest, Sha256};

#[test]
fn genuine_signature_receipt_binds_the_input_image_journal_and_seal() {
    let encoded = include_bytes!("../../../tests/fixtures/script-accelerated-receipt.bin");
    let input = include_bytes!("../../../tests/fixtures/script-accelerated-input.bin");
    let report: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/script-accelerated-receipt.json"
    ))
    .unwrap();
    assert_eq!(
        hex::encode(Sha256::digest(encoded)),
        report["receipt_sha256"].as_str().unwrap()
    );
    assert_eq!(
        hex::encode(Sha256::digest(input)),
        report["stdin_sha256"].as_str().unwrap()
    );
    let mut receipt: Receipt = bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(1024 * 1024)
        .reject_trailing_bytes()
        .deserialize(encoded)
        .unwrap();
    assert!(matches!(receipt.inner, InnerReceipt::Succinct(_)));
    assert_eq!(
        receipt.journal.bytes,
        Sha256::digest(&input[4..]).as_slice()
    );
    let image = risc0_zkvm::sha::Digest::try_from(
        hex::decode(report["image_id"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    let context = VerifierContext::default().with_dev_mode(false);
    receipt.verify_with_context(&context, image).unwrap();
    assert!(
        receipt
            .verify_with_context(&context, risc0_zkvm::sha::Digest::ZERO)
            .is_err()
    );
    receipt.journal.bytes[0] ^= 1;
    assert!(receipt.verify_with_context(&context, image).is_err());
    receipt.journal.bytes[0] ^= 1;
    let InnerReceipt::Succinct(inner) = &mut receipt.inner else {
        panic!("fixture is not a succinct receipt");
    };
    inner.seal[0] ^= 1;
    assert!(receipt.verify_with_context(&context, image).is_err());
}
