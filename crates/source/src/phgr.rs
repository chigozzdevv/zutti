use ark_bn254::{Bn254, Fq, Fq2, Fr, G1Affine, G2Affine};
use ark_ec::{AffineRepr, CurveGroup, pairing::Pairing, short_weierstrass::SWCurveConfig};
use ark_ff::{BigInteger, Field, PrimeField, Zero};
use num_bigint::BigUint;
use std::sync::OnceLock;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub(crate) enum Error {
    #[error("invalid historical Sprout proof encoding")]
    Encoding,
    #[error("invalid historical Sprout verifying key")]
    Key,
    #[error("historical Sprout proof does not verify")]
    Proof,
}
type Result<T> = core::result::Result<T, Error>;
type PreparedG2 = <Bn254 as Pairing>::G2Prepared;

#[derive(Debug)]
struct Key {
    alpha_a: G2Affine,
    alpha_b: G1Affine,
    alpha_c: G2Affine,
    gamma: G2Affine,
    gamma_beta_g1: G1Affine,
    gamma_beta_g2: G2Affine,
    z: G2Affine,
    ic: [G1Affine; 10],
}

pub(crate) fn verify(proof: &[u8], public: &[u8]) -> Result<()> {
    if proof.len() != 296 || public.len() != 272 {
        return Err(Error::Encoding);
    }
    static KEY: OnceLock<Result<Key>> = OnceLock::new();
    let key = KEY.get_or_init(read_key).as_ref().map_err(|_| Error::Key)?;
    let mut input = proof;
    let a = compressed_g1(&mut input)?;
    let a_prime = compressed_g1(&mut input)?;
    let b = compressed_g2(&mut input)?;
    let b_prime = compressed_g1(&mut input)?;
    let c = compressed_g1(&mut input)?;
    let c_prime = compressed_g1(&mut input)?;
    let k = compressed_g1(&mut input)?;
    let h = compressed_g1(&mut input)?;
    if !input.is_empty() {
        return Err(Error::Encoding);
    }
    let bits: Vec<bool> = public
        .iter()
        .flat_map(|byte| (0..8).rev().map(move |bit| byte & (1 << bit) != 0))
        .collect();
    let packed: Vec<Fr> = bits
        .chunks(253)
        .map(|chunk| {
            let mut value = Fr::zero();
            let mut power = Fr::from(1_u64);
            for bit in chunk {
                if *bit {
                    value += power;
                }
                power += power;
            }
            value
        })
        .collect();
    if packed.len() + 1 != key.ic.len() {
        return Err(Error::Encoding);
    }
    let mut acc = key.ic[0].into_group();
    for (base, scalar) in key.ic[1..].iter().zip(packed) {
        acc += base.mul_bigint(scalar.into_bigint());
    }
    let a_acc = (a.into_group() + acc).into_affine();
    let b = PreparedG2::from(b);
    let g2 = PreparedG2::from(G2Affine::generator());
    let valid = Bn254::multi_pairing([a, -a_prime], [key.alpha_a.into(), g2.clone()]).is_zero()
        && Bn254::multi_pairing([key.alpha_b, -b_prime], [b.clone(), g2.clone()]).is_zero()
        && Bn254::multi_pairing([c, -c_prime], [key.alpha_c.into(), g2.clone()]).is_zero()
        && Bn254::multi_pairing([a_acc, -h, -c], [b.clone(), key.z.into(), g2]).is_zero()
        && Bn254::multi_pairing(
            [k, -(a_acc + c).into_affine(), -key.gamma_beta_g1],
            [key.gamma.into(), key.gamma_beta_g2.into(), b],
        )
        .is_zero();
    if !valid {
        return Err(Error::Proof);
    }
    Ok(())
}

fn read_key() -> Result<Key> {
    let mut input = include_bytes!("../parameters/sprout-phgr.vk").as_slice();
    if take(&mut input, 10)? != b"ZUTTIPHG\x01\x00" {
        return Err(Error::Key);
    }
    let alpha_a = affine_g2(&mut input)?;
    let alpha_b = affine_g1(&mut input)?;
    let alpha_c = affine_g2(&mut input)?;
    let gamma = affine_g2(&mut input)?;
    let gamma_beta_g1 = affine_g1(&mut input)?;
    let gamma_beta_g2 = affine_g2(&mut input)?;
    let z = affine_g2(&mut input)?;
    let mut ic = [G1Affine::identity(); 10];
    for point in &mut ic {
        *point = affine_g1(&mut input)?;
    }
    if !input.is_empty() {
        return Err(Error::Key);
    }
    Ok(Key {
        alpha_a,
        alpha_b,
        alpha_c,
        gamma,
        gamma_beta_g1,
        gamma_beta_g2,
        z,
        ic,
    })
}
fn take<'a>(input: &mut &'a [u8], length: usize) -> Result<&'a [u8]> {
    let (head, tail) = input.split_at_checked(length).ok_or(Error::Encoding)?;
    *input = tail;
    Ok(head)
}
fn field(input: &mut &[u8]) -> Result<Fq> {
    let bytes = take(input, 32)?;
    if BigUint::from_bytes_be(bytes) >= BigUint::from_bytes_be(&Fq::MODULUS.to_bytes_be()) {
        return Err(Error::Encoding);
    }
    Ok(Fq::from_be_bytes_mod_order(bytes))
}
fn affine_g1(input: &mut &[u8]) -> Result<G1Affine> {
    let point = G1Affine::new_unchecked(field(input)?, field(input)?);
    if point.is_zero() || !point.is_on_curve() || !point.is_in_correct_subgroup_assuming_on_curve()
    {
        return Err(Error::Encoding);
    }
    Ok(point)
}
fn affine_g2(input: &mut &[u8]) -> Result<G2Affine> {
    let point = G2Affine::new_unchecked(
        Fq2::new(field(input)?, field(input)?),
        Fq2::new(field(input)?, field(input)?),
    );
    if point.is_zero() || !point.is_on_curve() || !point.is_in_correct_subgroup_assuming_on_curve()
    {
        return Err(Error::Encoding);
    }
    Ok(point)
}
fn compressed_g1(input: &mut &[u8]) -> Result<G1Affine> {
    let flag = take(input, 1)?[0];
    if flag & !1 != 2 {
        return Err(Error::Encoding);
    }
    let x = field(input)?;
    let mut y = (x.square() * x + ark_bn254::g1::Config::COEFF_B)
        .sqrt()
        .ok_or(Error::Encoding)?;
    if y.into_bigint().is_odd() != (flag & 1 != 0) {
        y = -y;
    }
    let point = G1Affine::new_unchecked(x, y);
    if point.is_zero() || !point.is_on_curve() || !point.is_in_correct_subgroup_assuming_on_curve()
    {
        return Err(Error::Encoding);
    }
    Ok(point)
}
fn compressed_g2(input: &mut &[u8]) -> Result<G2Affine> {
    let flag = take(input, 1)?[0];
    if flag & !1 != 10 {
        return Err(Error::Encoding);
    }
    let combined = BigUint::from_bytes_be(take(input, 64)?);
    let modulus = BigUint::from_bytes_be(&Fq::MODULUS.to_bytes_be());
    let c0 = &combined % &modulus;
    let c1 = &combined / &modulus;
    if c1 >= modulus {
        return Err(Error::Encoding);
    }
    let x = Fq2::new(
        Fq::from_be_bytes_mod_order(&c0.to_bytes_be()),
        Fq::from_be_bytes_mod_order(&c1.to_bytes_be()),
    );
    let mut y = (x.square() * x + ark_bn254::g2::Config::COEFF_B)
        .sqrt()
        .ok_or(Error::Encoding)?;
    let gt = |left: Fq2, right: Fq2| {
        (left.c1.into_bigint(), left.c0.into_bigint())
            > (right.c1.into_bigint(), right.c0.into_bigint())
    };
    if gt(y, -y) != (flag & 1 != 0) {
        y = -y;
    }
    let point = G2Affine::new_unchecked(x, y);
    if point.is_zero() || !point.is_on_curve() || !point.is_in_correct_subgroup_assuming_on_curve()
    {
        return Err(Error::Encoding);
    }
    Ok(point)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn canonical_ceremony_key_has_valid_subgroup_points_and_nine_inputs() {
        assert_eq!(read_key().unwrap().ic.len(), 10);
    }
    #[test]
    fn zero_noncanonical_and_truncated_points_are_rejected() {
        assert!(verify(&[0; 296], &[0; 272]).is_err());
        let mut bad = [255; 33];
        bad[0] = 2;
        assert!(compressed_g1(&mut bad.as_slice()).is_err());
        assert!(compressed_g2(&mut [10; 64].as_slice()).is_err());
        assert!(affine_g1(&mut [0; 64].as_slice()).is_err());
    }
    #[test]
    fn historical_proof_rejects_changed_public_inputs_and_curve_points() {
        let block = crate::block::ParsedBlock::parse(
            &hex::decode(include_str!("../../../tests/fixtures/block-396.txt").trim()).unwrap(),
            &zcash_protocol::consensus::MAIN_NETWORK,
        )
        .unwrap();
        let bundle = block
            .transactions()
            .iter()
            .find_map(|tx| tx.sprout_bundle())
            .unwrap();
        let js = &bundle.joinsplits[0];
        let hsig = blake2b_simd::Params::new()
            .hash_length(32)
            .personal(b"ZcashComputehSig")
            .to_state()
            .update(js.random_seed())
            .update(&js.nullifiers()[0])
            .update(&js.nullifiers()[1])
            .update(&bundle.joinsplit_pubkey)
            .finalize();
        let mut public = Vec::new();
        public.extend_from_slice(js.anchor());
        public.extend_from_slice(hsig.as_bytes());
        for index in 0..2 {
            public.extend_from_slice(&js.nullifiers()[index]);
            public.extend_from_slice(&js.macs()[index]);
        }
        for cm in js.commitments() {
            public.extend_from_slice(cm);
        }
        public.extend_from_slice(&i64::from(js.vpub_old()).to_le_bytes());
        public.extend_from_slice(&i64::from(js.vpub_new()).to_le_bytes());
        let mut serialized = Vec::new();
        js.write(&mut serialized).unwrap();
        let proof = &serialized[304..600];
        verify(proof, &public).unwrap();
        public[0] ^= 1;
        assert!(verify(proof, &public).is_err());
        public[0] ^= 1;
        let mut changed = proof.to_vec();
        changed[0] ^= 1;
        assert!(verify(&changed, &public).is_err());
    }
}
