use risc0_zkvm::{
    Groth16Receipt, Groth16ReceiptVerifierParameters, InnerReceipt, MaybePruned, Receipt,
    ReceiptClaim, VerifierContext, guest::env, sha::Digestible,
};

pub fn verify(image: [u8; 32], journal: Vec<u8>) {
    let mut seal = [0; 260];
    env::read_slice(&mut seal);
    let parameters = Groth16ReceiptVerifierParameters::default().digest();
    assert_eq!(
        &seal[..4],
        &parameters.as_bytes()[..4],
        "unsupported receipt parameters"
    );
    let claim = ReceiptClaim::ok(image, journal.clone());
    let receipt = Receipt::new(
        InnerReceipt::Groth16(Groth16Receipt::new(
            seal[4..].to_vec(),
            MaybePruned::Value(claim),
            parameters,
        )),
        journal,
    );
    receipt
        .verify_with_context(&VerifierContext::default().with_dev_mode(false), image)
        .expect("invalid predecessor Groth16 proof");
}
