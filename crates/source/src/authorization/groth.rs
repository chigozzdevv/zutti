use super::{Error, Result};
use bls12_381::Scalar;

#[cfg(feature = "windowed-inputs")]
#[path = "groth/windowed.rs"]
mod windowed;
#[cfg(feature = "windowed-inputs")]
pub(super) use windowed::{PreparedKey, check_proof, read_key};

#[cfg(all(feature = "accelerated-proofs", not(feature = "windowed-inputs")))]
use bellman_legacy::groth16 as backend;
#[cfg(not(feature = "accelerated-proofs"))]
use bls12_381::Bls12;
#[cfg(all(feature = "accelerated-proofs", not(feature = "windowed-inputs")))]
use bls12_381_legacy::Bls12;
#[cfg(not(feature = "accelerated-proofs"))]
use groth16 as backend;

#[cfg(not(feature = "windowed-inputs"))]
pub(super) type PreparedKey = backend::PreparedVerifyingKey<Bls12>;

#[cfg(not(feature = "windowed-inputs"))]
pub(super) fn read_key(bytes: &[u8]) -> Result<PreparedKey> {
    let mut remaining = bytes;
    let key = backend::VerifyingKey::read(&mut remaining).map_err(|_| Error::Key)?;
    if !remaining.is_empty() {
        return Err(Error::Key);
    }
    Ok(backend::prepare_verifying_key(&key))
}

#[cfg(not(feature = "windowed-inputs"))]
pub(super) fn check_proof(key: &PreparedKey, bytes: &[u8; 192], inputs: &[Scalar]) -> Result<()> {
    let proof = backend::Proof::read(bytes.as_slice()).map_err(|_| Error::Key)?;
    #[cfg(feature = "accelerated-proofs")]
    let converted = inputs
        .iter()
        .map(|input| {
            Option::from(bls12_381_legacy::Scalar::from_bytes(&input.to_bytes())).ok_or(Error::Key)
        })
        .collect::<Result<Vec<_>>>()?;
    #[cfg(feature = "accelerated-proofs")]
    let inputs = converted.as_slice();
    backend::verify_proof(key, &proof, inputs).map_err(|_| Error::Key)
}

#[cfg(all(test, feature = "accelerated-proofs"))]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn reference(bytes: &[u8; 192], inputs: &[Scalar]) -> bool {
        let key = groth16::VerifyingKey::<bls12_381::Bls12>::read(
            include_bytes!("../../parameters/sapling-output.vk").as_slice(),
        )
        .unwrap();
        let key = groth16::prepare_verifying_key(&key);
        groth16::Proof::read(bytes.as_slice())
            .ok()
            .is_some_and(|proof| groth16::verify_proof(&key, &proof, inputs).is_ok())
    }

    #[test]
    fn compatible_verifier_matches_real_outputs_and_rejects_changed_statements() {
        let fixtures: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../benchmarks/authorization/sapling-outputs.json"
        ))
        .unwrap();
        let key = read_key(include_bytes!("../../parameters/sapling-output.vk")).unwrap();
        for item in fixtures["proofs"].as_array().unwrap() {
            let proof: [u8; 192] = hex::decode(item["proof"].as_str().unwrap())
                .unwrap()
                .try_into()
                .unwrap();
            let inputs = item["inputs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| {
                    let bytes = hex::decode(value.as_str().unwrap())
                        .unwrap()
                        .try_into()
                        .unwrap();
                    Option::<Scalar>::from(Scalar::from_bytes(&bytes)).unwrap()
                })
                .collect::<Vec<_>>();
            assert!(reference(&proof, &inputs));
            check_proof(&key, &proof, &inputs).unwrap();
            for index in 0..inputs.len() {
                let mut changed = inputs.clone();
                changed[index] += Scalar::from(1);
                assert!(!reference(&proof, &changed));
                assert!(check_proof(&key, &proof, &changed).is_err());
            }
            assert!(check_proof(&key, &proof, &inputs[1..]).is_err());
            for (offset, length) in [(0, 48), (48, 96), (144, 48)] {
                let mut changed = proof;
                changed[offset..offset + length].fill(0);
                changed[offset] = 0xc0;
                assert!(!reference(&changed, &inputs));
                assert!(check_proof(&key, &changed, &inputs).is_err());
            }
        }
    }

    #[test]
    fn malformed_keys_and_proofs_are_rejected() {
        let bytes = include_bytes!("../../parameters/sapling-output.vk");
        let mut trailing = bytes.to_vec();
        trailing.push(0);
        assert!(read_key(&trailing).is_err());
        assert!(read_key(&bytes[..bytes.len() - 1]).is_err());
        let key = read_key(bytes).unwrap();
        for proof in [[0; 192], [0xff; 192]] {
            assert!(!reference(&proof, &[Scalar::from(1); 5]));
            assert!(check_proof(&key, &proof, &[Scalar::from(1); 5]).is_err());
        }
    }
}
