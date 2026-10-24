use risc0_zkvm::{FakeReceipt, InnerReceipt, Receipt, ReceiptClaim};
use std::path::PathBuf;
use zcash_protocol::consensus::NetworkType;
use zutti_protocol::{Digest, encoding::Canonical};
use zutti_source::spv::{HEADER_BYTES, HeaderClaim, Policy, Witness};
use zutti_spv_prototype::{check_previous, read_receipt, verify};

fn genuine_genesis() -> (Receipt, Digest, Vec<u8>) {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/genesis-v2");
    let receipt = read_receipt(&directory.join("receipt.bin")).unwrap();
    let image = Digest(
        hex::decode("5b1db5eb5db64f783127cb2083c232a06e34c8a238e1aacb0a6c165c6b26b8bd")
            .unwrap()
            .try_into()
            .unwrap(),
    );
    let journal = std::fs::read(directory.join("journal.bin")).unwrap();
    (receipt, image, journal)
}

#[test]
fn genuine_genesis_receipt_binds_the_program_and_journal() {
    let (receipt, image, journal) = genuine_genesis();
    assert!(matches!(receipt.inner, InnerReceipt::Composite(_)));
    verify(&receipt, image, &journal).unwrap();
    let prefix = HeaderClaim::from_bytes(&journal).unwrap();
    assert_eq!(prefix.height().unwrap(), 0);
    assert_eq!(prefix.policy().program(), image);
    let mut wrong_image = image;
    wrong_image.0[0] ^= 1;
    assert!(verify(&receipt, wrong_image, &journal).is_err());
    let mut wrong_journal = journal;
    wrong_journal[0] ^= 1;
    assert!(verify(&receipt, image, &wrong_journal).is_err());
    let mut changed = receipt.clone();
    changed.journal.bytes = wrong_journal;
    assert!(verify(&changed, image, &changed.journal.bytes).is_err());
    let mut changed = receipt;
    let InnerReceipt::Composite(inner) = &mut changed.inner else {
        panic!("expected a composite receipt");
    };
    inner.segments[0].seal[0] ^= 1;
    assert!(verify(&changed, image, &changed.journal.bytes).is_err());
}

#[test]
fn genuine_predecessor_cannot_authorize_a_different_policy() {
    let (receipt, image, journal) = genuine_genesis();
    let previous = HeaderClaim::from_bytes(&journal).unwrap();
    let first =
        hex::decode(include_str!("../../../../tests/fixtures/spv/testnet-1.hex").trim()).unwrap();
    let witness = Witness::Headers {
        previous,
        headers: vec![first[..HEADER_BYTES].to_vec()],
    };
    check_previous(&witness, Some(&receipt), image).unwrap();
    let genesis =
        hex::decode(include_str!("../../../../tests/fixtures/block-test-0.txt").trim()).unwrap();
    let changed = Witness::Headers {
        previous: HeaderClaim::genesis(
            Policy::new(image, NetworkType::Test, 2, 3, 4).unwrap(),
            &genesis[..HEADER_BYTES],
        )
        .unwrap(),
        headers: vec![first[..HEADER_BYTES].to_vec()],
    };
    assert!(check_previous(&changed, Some(&receipt), image).is_err());
}

fn prefix() -> HeaderClaim {
    let bytes =
        hex::decode(include_str!("../../../../tests/fixtures/block-test-0.txt").trim()).unwrap();
    HeaderClaim::genesis(
        Policy::new(Digest([1; 32]), NetworkType::Test, 1, 3, 4).unwrap(),
        &bytes[..HEADER_BYTES],
    )
    .unwrap()
}

#[test]
fn fake_receipts_are_rejected_even_with_a_matching_claim_and_journal() {
    let previous = prefix();
    let journal = previous.to_bytes().unwrap();
    let receipt = Receipt::new(
        InnerReceipt::Fake(FakeReceipt::new(ReceiptClaim::ok(
            [1_u8; 32],
            journal.clone(),
        ))),
        journal.clone(),
    );
    assert!(verify(&receipt, Digest([1; 32]), &journal).is_err());
}

#[test]
fn continuation_requires_genuine_evidence_for_the_exact_predecessor() {
    let first =
        hex::decode(include_str!("../../../../tests/fixtures/spv/testnet-1.hex").trim()).unwrap();
    let witness = Witness::Headers {
        previous: prefix(),
        headers: vec![first[..HEADER_BYTES].to_vec()],
    };
    assert!(check_previous(&witness, None, Digest([1; 32])).is_err());
    assert!(check_previous(&witness, None, Digest([9; 32])).is_err());
}

#[test]
fn legacy_receipt_verifies_only_its_old_statement_and_is_not_a_v2_prefix() {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/genesis");
    let receipt = read_receipt(&directory.join("receipt.bin")).unwrap();
    let image = Digest(
        hex::decode("de286cb10a4a1e1e540a9f154962d4e9235500ccee85c653781f52ff82da39f4")
            .unwrap()
            .try_into()
            .unwrap(),
    );
    verify(&receipt, image, &receipt.journal.bytes).unwrap();
    assert!(HeaderClaim::from_bytes(&receipt.journal.bytes).is_err());
}
