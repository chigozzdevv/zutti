use crate::{Result, verify};
use risc0_zkvm::{
    ExternalProver, Groth16ReceiptVerifierParameters, InnerReceipt, Prover, ProverOpts, Receipt,
    sha::Digestible,
};
use zutti_protocol::Digest;

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
