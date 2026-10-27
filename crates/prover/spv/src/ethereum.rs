use crate::{Result, verify};
use risc0_zkvm::{
    ExternalProver, Groth16Receipt, Groth16ReceiptVerifierParameters, InnerReceipt, MaybePruned,
    Prover, ProverOpts, Receipt, ReceiptClaim, sha::Digestible,
};
use zutti_protocol::Digest;
use zutti_protocol::encoding::Canonical;
use zutti_source::spv::{AdmissionClaim, AdmissionKind, ChainPoint};

fn admission_calldata(seal: &[u8], claim: &AdmissionClaim) -> Result<Vec<u8>> {
    claim.to_bytes()?;
    if claim.kind != AdmissionKind::Advance || seal.len() != 260 {
        return Err("registry submission requires an advance and a Groth16 seal".into());
    }
    let mut data = vec![0x85, 0x73, 0xd7, 0x50];
    let word = |value: u32| {
        let mut bytes = [0; 32];
        bytes[28..].copy_from_slice(&value.to_be_bytes());
        bytes
    };
    data.extend_from_slice(&word(15 * 32));
    data.extend_from_slice(&word(claim.kind as u32));
    data.extend_from_slice(&claim.policy.0);
    data.extend_from_slice(&claim.previous.0);
    for ChainPoint {
        prefix,
        height,
        hash,
        timestamp,
        work,
    } in [&claim.stable, &claim.head]
    {
        data.extend_from_slice(&prefix.0);
        data.extend_from_slice(&word(*height));
        data.extend_from_slice(&hash.0);
        data.extend_from_slice(&word(*timestamp));
        data.extend_from_slice(work);
    }
    data.extend_from_slice(&word(claim.maximum_time));
    data.extend_from_slice(&word(seal.len() as u32));
    data.extend_from_slice(seal);
    data.resize(4 + (data.len() - 4).next_multiple_of(32), 0);
    Ok(data)
}

pub fn admission_call(
    receipt: &Receipt,
    image: Digest,
    policy: Digest,
    registry: [u8; 20],
) -> Result<serde_json::Value> {
    if registry == [0; 20] {
        return Err("registry address is zero".into());
    }
    let claim = AdmissionClaim::from_bytes(&receipt.journal.bytes)?;
    if claim.policy != policy {
        return Err("source admission policy differs from deployment".into());
    }
    let seal = seal(receipt, image, &receipt.journal.bytes)?;
    let data = admission_calldata(&seal, &claim)?;
    use sha2::{Digest as _, Sha256};
    Ok(serde_json::json!({
        "schema": 1,
        "chain_id": "11155111",
        "to": format!("0x{}", hex::encode(registry)),
        "data": format!("0x{}", hex::encode(data)),
        "value": "0x0",
        "image_id": image.to_string(),
        "source_policy": policy.to_string(),
        "journal_sha256": hex::encode(Sha256::digest(&receipt.journal.bytes)),
        "proof_verified": true,
        "transaction_submitted": false
    }))
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_calldata_matches_solidity_abi_vector() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/spv/admission-calldata.json"
        ))
        .unwrap();
        let claim =
            AdmissionClaim::from_bytes(&hex::decode(fixture["journal"].as_str().unwrap()).unwrap())
                .unwrap();
        let seal = hex::decode(fixture["seal"].as_str().unwrap()).unwrap();
        let encoded = admission_calldata(&seal, &claim).unwrap();
        assert_eq!(hex::encode(&encoded), fixture["calldata"].as_str().unwrap());
        assert_eq!(encoded.len(), 804);
    }
}
