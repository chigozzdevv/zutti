use super::{Error, Result, Scalar};
use bls12_381_legacy::{G1Affine, G1Projective, G2Prepared, Gt, multi_miller_loop, pairing};

pub(crate) struct PreparedKey {
    alpha_beta: Gt,
    gamma: G2Prepared,
    delta: G2Prepared,
    constant: G1Affine,
    windows: Vec<[G1Affine; 16]>,
}
impl core::fmt::Debug for PreparedKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PreparedKey")
            .field("inputs", &self.windows.len())
            .finish_non_exhaustive()
    }
}
impl PreparedKey {
    fn accumulate(&self, inputs: &[Scalar]) -> Result<G1Affine> {
        if inputs.len() != self.windows.len() {
            return Err(Error::Key);
        }
        let scalars = inputs.iter().map(Scalar::to_bytes).collect::<Vec<_>>();
        let mut acc = G1Projective::identity();
        for window in (0..64).rev() {
            for _ in 0..4 {
                acc = acc.double();
            }
            for (bytes, points) in scalars.iter().zip(&self.windows) {
                let digit = (bytes[window / 2] >> (4 * (window % 2))) & 15;
                if digit != 0 {
                    acc += points[usize::from(digit)];
                }
            }
        }
        Ok(G1Affine::from(acc + self.constant))
    }
}

pub(crate) fn read_key(bytes: &[u8]) -> Result<PreparedKey> {
    let mut remaining = bytes;
    let key =
        bellman_legacy::groth16::VerifyingKey::<bls12_381_legacy::Bls12>::read(&mut remaining)
            .map_err(|_| Error::Key)?;
    if !remaining.is_empty() || !(2..=17).contains(&key.ic.len()) {
        return Err(Error::Key);
    }
    let mut multiples = vec![G1Projective::identity(); (key.ic.len() - 1) * 16];
    for (base, window) in key.ic[1..].iter().zip(multiples.chunks_exact_mut(16)) {
        for digit in 1..16 {
            window[digit] = window[digit - 1] + base;
        }
    }
    let mut affine = vec![G1Affine::identity(); multiples.len()];
    G1Projective::batch_normalize(&multiples, &mut affine);
    Ok(PreparedKey {
        alpha_beta: pairing(&key.alpha_g1, &key.beta_g2),
        gamma: G2Prepared::from(-key.gamma_g2),
        delta: G2Prepared::from(-key.delta_g2),
        constant: key.ic[0],
        windows: affine
            .chunks_exact(16)
            .map(|points| core::array::from_fn(|i| points[i]))
            .collect(),
    })
}

pub(crate) fn check_proof(key: &PreparedKey, bytes: &[u8; 192], inputs: &[Scalar]) -> Result<()> {
    let proof = bellman_legacy::groth16::Proof::<bls12_381_legacy::Bls12>::read(bytes.as_slice())
        .map_err(|_| Error::Key)?;
    let input = key.accumulate(inputs)?;
    let b = G2Prepared::from(proof.b);
    let result = multi_miller_loop(&[(&proof.a, &b), (&input, &key.gamma), (&proof.c, &key.delta)])
        .final_exponentiation();
    if result != key.alpha_beta {
        return Err(Error::Key);
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn joint_windows_match_independent_scalar_multiplication_for_all_consensus_keys() {
        for bytes in [
            include_bytes!("../../../parameters/sapling-output.vk").as_slice(),
            include_bytes!("../../../parameters/sapling-spend.vk").as_slice(),
            include_bytes!("../../../parameters/sprout-groth16.vk").as_slice(),
        ] {
            let key = read_key(bytes).unwrap();
            let original =
                bellman_legacy::groth16::VerifyingKey::<bls12_381_legacy::Bls12>::read(bytes)
                    .unwrap();
            let cases = [
                Scalar::from(0),
                Scalar::from(1),
                Scalar::from(15),
                Scalar::from(16),
                Scalar::from(255),
                -Scalar::from(1),
            ];
            for shift in 0..cases.len() {
                let inputs = (0..key.windows.len())
                    .map(|i| cases[(i + shift) % cases.len()])
                    .collect::<Vec<_>>();
                let mut expected = G1Projective::from(original.ic[0]);
                for (base, input) in original.ic[1..].iter().zip(&inputs) {
                    let scalar = Option::<bls12_381_legacy::Scalar>::from(
                        bls12_381_legacy::Scalar::from_bytes(&input.to_bytes()),
                    )
                    .unwrap();
                    expected += base * scalar;
                }
                assert_eq!(key.accumulate(&inputs).unwrap(), G1Affine::from(expected));
            }
            assert!(key.accumulate(&[]).is_err());
        }
    }
}
