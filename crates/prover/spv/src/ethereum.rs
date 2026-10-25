use crate::{Result, verify};
use risc0_zkvm::{
    ExternalProver, Groth16Receipt, Groth16ReceiptVerifierParameters, InnerReceipt, MaybePruned,
    Prover, ProverOpts, Receipt, ReceiptClaim, sha::Digestible,
};
use zutti_protocol::Digest;

pub fn from_seal(image: Digest, journal: &[u8], seal: &[u8]) -> Result<Receipt> {
    let parameters = Groth16ReceiptVerifierParameters::default().digest();
    if seal.len() != 260 || seal[..4] != parameters.as_bytes()[..4] {
        return Err("unsupported Ethereum receipt seal".into());
    }
    let receipt = Receipt::new(
        InnerReceipt::Groth16(Groth16Receipt::new(
            seal[4..].to_vec(),
            MaybePruned::Value(ReceiptClaim::ok(image.0, journal.to_vec())),
            parameters,
        )),
        journal.to_vec(),
    );
    verify(&receipt, image, journal)?;
    Ok(receipt)
}

pub fn seal(receipt: &Receipt, image: Digest, journal: &[u8]) -> Result<Vec<u8>> {
    verify(receipt, image, journal)?;
    let InnerReceipt::Groth16(inner) = &receipt.inner else {
        return Err("Ethereum export requires a genuine Groth16 receipt".into());
    };
    let expected = Groth16ReceiptVerifierParameters::default().digest();
    if inner.verifier_parameters != expected || inner.seal.len() != 256 {
        return Err("unsupported Ethereum receipt parameters".into());
    }
    let mut seal = expected.as_bytes()[..4].to_vec();
    seal.extend_from_slice(&inner.seal);
    Ok(seal)
}
pub fn wrap(receipt: &Receipt, image: Digest, journal: &[u8]) -> Result<(Receipt, Vec<u8>)> {
    verify(receipt, image, journal)?;
    let wrapped = ExternalProver::new("zutti-spv-wrap", "r0vm")
        .compress(&ProverOpts::groth16().with_dev_mode(false), receipt)?;
    let seal = seal(&wrapped, image, journal)?;
    Ok((wrapped, seal))
}
