use risc0_zkvm::{
    Groth16Receipt, Groth16ReceiptVerifierParameters, MaybePruned, ReceiptClaim, VerifierContext,
    sha::Digestible,
};

pub fn verify(image: [u8; 32], journal: &[u8], seal: &[u8]) -> Result<(), &'static str> {
    let parameters = Groth16ReceiptVerifierParameters::default().digest();
    if seal.len() != 260 || seal[..4] != parameters.as_bytes()[..4] {
        return Err("unsupported dependency seal");
    }
    Groth16Receipt::new(
        seal[4..].to_vec(),
        MaybePruned::Value(ReceiptClaim::ok(image, journal.to_vec())),
        parameters,
    )
    .verify_integrity_with_context(&VerifierContext::default().with_dev_mode(false))
    .map_err(|_| "invalid dependency proof")
}

#[cfg(target_os = "zkvm")]
pub fn read_and_verify(image: [u8; 32], journal: &[u8]) {
    let mut seal = [0; 260];
    risc0_zkvm::guest::env::read_slice(&mut seal);
    verify(image, journal, &seal).expect("unverified SPV dependency");
}
