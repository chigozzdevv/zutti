use risc0_zkvm::{
    Groth16Receipt, Groth16ReceiptVerifierParameters, InnerReceipt, MaybePruned, Receipt,
    ReceiptClaim, sha::Digestible,
};
use zutti_protocol::Digest;
use zutti_spv_prototype::ethereum::{admission_call, seal, wrap};

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

#[test]
fn a_genuine_unrelated_receipt_cannot_become_source_admission() {
    let (image, _, receipt, _) = genuine();
    assert!(admission_call(&receipt, image, Digest([1; 32]), [2; 20]).is_err());
    assert!(admission_call(&receipt, image, Digest([1; 32]), [0; 20]).is_err());
}

#[test]
fn genuine_active_admission_binds_the_verified_source_prefix_and_exported_policy() {
    use std::{fs, path::Path};
    use zutti_protocol::encoding::Canonical;
    use zutti_source::spv::{AdmissionClaim, HeaderClaim};
    use zutti_spv_prototype::{image, read_receipt, verify};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../deploy/source/testnet");
    let profile: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("profile.json")).unwrap()).unwrap();
    let source = read_receipt(&root.join("source-receipt.bin")).unwrap();
    let source_image = image(&fs::read(root.join("source-program.elf")).unwrap()).unwrap();
    verify(&source, source_image, &source.journal.bytes).unwrap();
    let header = HeaderClaim::from_bytes(&source.journal.bytes).unwrap();
    let admission = read_receipt(&root.join("admission-receipt.bin")).unwrap();
    let admission_image = image(&fs::read(root.join("admission-program.elf")).unwrap()).unwrap();
    let claim = AdmissionClaim::from_bytes(&admission.journal.bytes).unwrap();
    assert_eq!(claim.stable.prefix, header.id().unwrap());
    assert_eq!(claim.stable.height, header.height().unwrap());
    assert_eq!(
        claim.head.height - claim.stable.height,
        header.policy().confirmations()
    );
    assert_eq!(claim.policy.to_string(), profile["policy"]["id"]);
    let bundle = admission_call(&admission, admission_image, claim.policy, [22; 20]).unwrap();
    assert_eq!(bundle["proof_verified"], true);
    assert!(bundle["data"].as_str().unwrap().starts_with("0x8573d750"));
    assert!(admission_call(&admission, admission_image, Digest([253; 32]), [22; 20]).is_err());
    let mut changed = admission;
    changed.journal.bytes[11] ^= 1;
    assert!(admission_call(&changed, admission_image, claim.policy, [22; 20]).is_err());
}

#[test]
fn payment_admission_composes_the_genuine_swap_receipt_without_revalidating_its_header_range() {
    use std::{fs, path::Path};
    use zutti_protocol::encoding::Canonical;
    use zutti_source::spv::{
        AdmissionClaim, AdmissionWitness, HEADER_BYTES, HeaderClaim, WindowClaim,
    };
    use zutti_spv_prototype::{read_receipt, verify};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../deploy");
    let previous =
        HeaderClaim::from_bytes(&fs::read(root.join("source/testnet/source-journal.bin")).unwrap())
            .unwrap();
    let headers = fs::read(root.join("ethereum/payment-window-headers.bin")).unwrap();
    let stable = previous
        .advance(
            &headers
                .chunks_exact(HEADER_BYTES)
                .map(<[u8]>::to_vec)
                .collect::<Vec<_>>(),
        )
        .unwrap();
    let receipt = read_receipt(&root.join("ethereum/window-receipt.bin")).unwrap();
    let window = WindowClaim::from_bytes(&receipt.journal.bytes).unwrap();
    let witness = AdmissionWitness::Payment {
        previous,
        stable,
        window: Box::new(window),
        confirmations: fs::read(root.join("ethereum/payment-window-confirmations.bin"))
            .unwrap()
            .chunks_exact(HEADER_BYTES)
            .map(<[u8]>::to_vec)
            .collect(),
    };
    let required = witness.required_journal().unwrap();
    let program = witness.predecessor().policy().program();
    verify(&receipt, program, &required).unwrap();
    let original = AdmissionClaim::from_bytes(
        &fs::read(root.join("ethereum/payment-admission-journal.bin")).unwrap(),
    )
    .unwrap();
    assert_eq!(witness.evaluate().unwrap(), original);
    let unrelated = read_receipt(&root.join("source/testnet/source-receipt.bin")).unwrap();
    assert!(verify(&unrelated, program, &required).is_err());
    let mut changed = required;
    *changed.last_mut().unwrap() ^= 1;
    assert!(verify(&receipt, program, &changed).is_err());
}

#[test]
fn optimized_admission_has_its_own_genuine_receipt_for_the_original_statement() {
    use std::{fs, path::Path};
    use zutti_protocol::encoding::Canonical;
    use zutti_source::spv::{AdmissionClaim, WindowClaim};
    use zutti_spv_prototype::{image, read_receipt, verify};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../deploy");
    let directory = root.join("source/optimized-admission");
    let program = image(&fs::read(directory.join("program.elf")).unwrap()).unwrap();
    let receipt = read_receipt(&directory.join("receipt.bin")).unwrap();
    let journal = fs::read(directory.join("journal.bin")).unwrap();
    verify(&receipt, program, &journal).unwrap();
    assert_eq!(
        journal,
        fs::read(root.join("ethereum/payment-admission-journal.bin")).unwrap()
    );
    let claim = AdmissionClaim::from_bytes(&journal).unwrap();
    let window =
        WindowClaim::from_bytes(&fs::read(root.join("ethereum/window-journal.bin")).unwrap())
            .unwrap();
    assert_eq!(claim.previous, window.prefix);
    assert_eq!(claim.stable.prefix, window.checkpoint);
    assert_eq!(claim.policy, window.policy);
    assert_eq!(claim.head.height - claim.stable.height, 10);
    let original =
        image(&fs::read(root.join("source/testnet/admission-program.elf")).unwrap()).unwrap();
    assert_ne!(program, original);
    assert!(verify(&receipt, original, &journal).is_err());
    let original_receipt =
        read_receipt(&root.join("ethereum/payment-admission-receipt.bin")).unwrap();
    assert!(verify(&original_receipt, program, &journal).is_err());
    let mut altered = journal;
    *altered.last_mut().unwrap() ^= 1;
    assert!(verify(&receipt, program, &altered).is_err());
}

#[test]
fn genuine_funded_deposit_receipt_binds_the_actual_pool_domain_and_value() {
    use std::{fs, path::Path};
    use zutti_protocol::{encoding::Canonical, pool::OperationKind};
    use zutti_spv_prototype::{image, read_receipt, verify};
    use zutti_wallet::spv_pool::SpvPoolClaim;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../deploy/ethereum");
    let deployment: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("sepolia.json")).unwrap()).unwrap();
    let receipt = read_receipt(&root.join("maker-deposit-receipt.bin")).unwrap();
    let program = image(&fs::read(root.join("pool-program.elf")).unwrap()).unwrap();
    verify(&receipt, program, &receipt.journal.bytes).unwrap();
    let claim = SpvPoolClaim::from_bytes(&receipt.journal.bytes).unwrap();
    assert_eq!(claim.pool.kind, OperationKind::Deposit);
    assert_eq!(claim.pool.value, 1_000_000);
    assert_eq!(claim.pool.domain.to_string(), deployment["domain"]);
    assert_eq!(claim.pool_program.to_string(), deployment["pool_image"]);
    let mut changed = claim;
    changed.pool.value += 1;
    assert!(verify(&receipt, program, &changed.to_bytes().unwrap()).is_err());
}

#[test]
fn genuine_reservation_receipt_preserves_the_bound_payment_and_deployment() {
    use std::{fs, path::Path};
    use zutti_protocol::{encoding::Canonical, pool::OperationKind};
    use zutti_spv_prototype::{image, read_receipt, verify};
    use zutti_wallet::{binding::PaymentBindingClaim, spv_pool::SpvPoolClaim};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../deploy/ethereum");
    let binding = read_receipt(&root.join("binding-receipt.bin")).unwrap();
    let binding_image = image(&fs::read(root.join("binding-program.elf")).unwrap()).unwrap();
    verify(&binding, binding_image, &binding.journal.bytes).unwrap();
    let binding = PaymentBindingClaim::from_bytes(&binding.journal.bytes).unwrap();
    let reservation = read_receipt(&root.join("reservation-receipt.bin")).unwrap();
    let program = image(&fs::read(root.join("pool-program.elf")).unwrap()).unwrap();
    verify(&reservation, program, &reservation.journal.bytes).unwrap();
    let claim = SpvPoolClaim::from_bytes(&reservation.journal.bytes).unwrap();
    assert_eq!(claim.pool.kind, OperationKind::Reserve);
    assert_eq!(claim.pool.domain, binding.domain());
    assert_eq!(claim.pool.order, binding.order());
    assert_eq!(claim.pool.reservation, binding.reservation());
    assert_eq!(claim.policy, binding.policy());
    assert_eq!(claim.binding_program, binding_image);
    assert_eq!(claim.pool_program, program);
    assert_eq!(claim.expiry, 4_547_968);
    let mut changed = claim;
    changed.pool.reservation = Digest([254; 32]);
    assert!(verify(&reservation, program, &changed.to_bytes().unwrap()).is_err());
}

#[test]
fn genuine_live_swap_receipts_bind_payment_admission_payout_and_withdrawal() {
    use std::{collections::BTreeMap, fs, path::Path};
    use zutti_protocol::{encoding::Canonical, pool::OperationKind};
    use zutti_source::spv::{AdmissionClaim, WindowClaim};
    use zutti_spv_prototype::{image, read_receipt, verify};
    use zutti_wallet::{binding::PaymentBindingClaim, spv_pool::SpvPoolClaim};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../deploy/ethereum");
    let source = root.join("../source/testnet");
    let pool_image = image(&fs::read(root.join("pool-program.elf")).unwrap()).unwrap();
    let source_image = image(&fs::read(source.join("source-program.elf")).unwrap()).unwrap();
    let binding_image = image(&fs::read(root.join("binding-program.elf")).unwrap()).unwrap();
    let admission_image = image(&fs::read(source.join("admission-program.elf")).unwrap()).unwrap();
    let mut receipts = BTreeMap::new();
    for (role, program) in [
        ("binding", binding_image),
        ("reservation", pool_image),
        ("window", source_image),
        ("payment-admission", admission_image),
        ("payout", pool_image),
        ("withdrawal", pool_image),
    ] {
        let receipt = read_receipt(&root.join(format!("{role}-receipt.bin"))).unwrap();
        verify(&receipt, program, &receipt.journal.bytes).unwrap();
        receipts.insert(role, receipt);
    }
    let binding = PaymentBindingClaim::from_bytes(&receipts["binding"].journal.bytes).unwrap();
    let reservation = SpvPoolClaim::from_bytes(&receipts["reservation"].journal.bytes).unwrap();
    let window = WindowClaim::from_bytes(&receipts["window"].journal.bytes).unwrap();
    let admission =
        AdmissionClaim::from_bytes(&receipts["payment-admission"].journal.bytes).unwrap();
    let mut payout = SpvPoolClaim::from_bytes(&receipts["payout"].journal.bytes).unwrap();
    let withdrawal = SpvPoolClaim::from_bytes(&receipts["withdrawal"].journal.bytes).unwrap();
    assert_eq!(reservation.pool.order, binding.order());
    assert_eq!(window.policy, binding.policy());
    assert_eq!(window.reservation, binding.source_reservation());
    assert_eq!(admission.stable.prefix, window.checkpoint);
    assert_eq!(admission.stable.height, window.height);
    assert_eq!(payout.policy, binding.policy());
    assert_eq!(payout.pool.domain, binding.domain());
    assert_eq!(payout.pool.kind, OperationKind::Settle);
    assert_eq!(payout.pool.checkpoint, window.checkpoint);
    assert_eq!(withdrawal.pool.domain, binding.domain());
    assert_eq!(withdrawal.pool.kind, OperationKind::Withdraw);
    assert_eq!(withdrawal.pool.value, 1_000_000);
    assert_eq!(
        hex::encode(withdrawal.pool.owner),
        "fa48895f139df6a9fbfaf85dbcfcd043c343ad3d"
    );
    payout.pool.checkpoint = Digest([253; 32]);
    assert!(verify(&receipts["payout"], pool_image, &payout.to_bytes().unwrap()).is_err());
}
