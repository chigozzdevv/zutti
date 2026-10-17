use bellman::{gadgets::multipack, groth16};
use bls12_381::{Bls12, Scalar};
use group::{Curve, GroupEncoding, ff::PrimeField};
use orchard::circuit::{OrchardCircuitVersion, VerifyingKey};
use std::sync::OnceLock;
use transparent::bundle::TxOut;
use zcash_primitives::transaction::Transaction;
use zcash_script::script::Code;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("invalid embedded consensus verifying key")]
    Key,
    #[error("cannot calculate the transaction signature digest")]
    Sighash,
    #[error("invalid Sapling spend proof or authorization")]
    SaplingSpend,
    #[error("invalid Sapling output proof")]
    SaplingOutput,
    #[error("invalid Sapling binding signature")]
    SaplingBinding,
    #[error("invalid Sprout JoinSplit signature or proof")]
    Sprout,
    #[error("invalid Orchard or Ironwood proof or authorization")]
    Orchard,
}
type Result<T> = core::result::Result<T, Error>;
type PreparedKey = groth16::PreparedVerifyingKey<Bls12>;

pub fn verify(tx: &Transaction, prevouts: &[TxOut]) -> Result<()> {
    if tx.sprout_bundle().is_none()
        && tx.sapling_bundle().is_none()
        && tx.orchard_bundle().is_none()
        && tx.ironwood_bundle().is_none()
    {
        return Ok(());
    }
    let sighash = crate::transaction::input_signature_hash(tx, prevouts, None, &Code(vec![]), 1)
        .map_err(|_| Error::Sighash)?;
    if let Some(bundle) = tx.sprout_bundle() {
        let key = ed25519_zebra::VerificationKey::try_from(bundle.joinsplit_pubkey)
            .map_err(|_| Error::Sprout)?;
        key.verify(
            &ed25519_zebra::Signature::from(bundle.joinsplit_sig),
            &sighash,
        )
        .map_err(|_| Error::Sprout)?;
        for js in &bundle.joinsplits {
            let mut h = blake2b_simd::Params::new()
                .hash_length(32)
                .personal(b"ZcashComputehSig")
                .to_state();
            h.update(js.random_seed());
            h.update(&js.nullifiers()[0]);
            h.update(&js.nullifiers()[1]);
            h.update(&bundle.joinsplit_pubkey);
            let h_sig = h.finalize();
            let mut public = Vec::with_capacity(272);
            public.extend_from_slice(js.anchor());
            public.extend_from_slice(h_sig.as_bytes());
            for index in 0..2 {
                public.extend_from_slice(&js.nullifiers()[index]);
                public.extend_from_slice(&js.macs()[index]);
            }
            for cm in js.commitments() {
                public.extend_from_slice(cm);
            }
            public.extend_from_slice(&i64::from(js.vpub_old()).to_le_bytes());
            public.extend_from_slice(&i64::from(js.vpub_new()).to_le_bytes());
            if let Some(proof) = js.groth_proof_bytes() {
                let inputs = multipack::compute_multipacking(&multipack::bytes_to_bits(&public));
                check_proof(sprout_key()?, proof, &inputs).map_err(|_| Error::Sprout)?;
            } else {
                let mut encoded = Vec::new();
                js.write(&mut encoded).map_err(|_| Error::Sprout)?;
                crate::phgr::verify(encoded.get(304..600).ok_or(Error::Sprout)?, &public)
                    .map_err(|_| Error::Sprout)?;
            }
        }
    }
    if let Some(bundle) = tx.sapling_bundle() {
        let mut sum = sapling::value::CommitmentSum::zero();
        for spend in bundle.shielded_spends() {
            let rk = Option::<jubjub::AffinePoint>::from(jubjub::AffinePoint::from_bytes(
                (*spend.rk()).into(),
            ))
            .ok_or(Error::SaplingSpend)?;
            if bool::from(rk.is_small_order())
                || spend.rk().verify(&sighash, spend.spend_auth_sig()).is_err()
            {
                return Err(Error::SaplingSpend);
            }
            let cv = spend.cv().as_inner().to_affine();
            let packed: Vec<Scalar> =
                multipack::compute_multipacking(&multipack::bytes_to_bits_le(&spend.nullifier().0));
            let inputs = [
                rk.get_u(),
                rk.get_v(),
                cv.get_u(),
                cv.get_v(),
                *spend.anchor(),
                packed[0],
                packed[1],
            ];
            check_proof(sapling_spend_key()?, spend.zkproof(), &inputs)
                .map_err(|_| Error::SaplingSpend)?;
            sum += spend.cv();
        }
        for output in bundle.shielded_outputs() {
            let epk = Option::<jubjub::ExtendedPoint>::from(jubjub::ExtendedPoint::from_bytes(
                &output.ephemeral_key().0,
            ))
            .ok_or(Error::SaplingOutput)?;
            if bool::from(epk.is_small_order()) {
                return Err(Error::SaplingOutput);
            }
            let cv = output.cv().as_inner().to_affine();
            let epk = epk.to_affine();
            let cmu = Option::<Scalar>::from(Scalar::from_repr(output.cmu().to_bytes()))
                .ok_or(Error::SaplingOutput)?;
            check_proof(
                sapling_output_key()?,
                output.zkproof(),
                &[cv.get_u(), cv.get_v(), epk.get_u(), epk.get_v(), cmu],
            )
            .map_err(|_| Error::SaplingOutput)?;
            sum -= output.cv();
        }
        sum.into_bvk(*bundle.value_balance())
            .verify(&sighash, &bundle.authorization().binding_sig)
            .map_err(|_| Error::SaplingBinding)?;
    }
    for bundle in [tx.orchard_bundle(), tx.ironwood_bundle()]
        .into_iter()
        .flatten()
    {
        for action in bundle.actions() {
            action
                .rk()
                .verify(&sighash, action.authorization())
                .map_err(|_| Error::Orchard)?;
        }
        bundle
            .binding_validating_key()
            .verify(&sighash, bundle.authorization().binding_signature())
            .map_err(|_| Error::Orchard)?;
        bundle
            .verify_proof(orchard_key(bundle.bundle_version().circuit_version()))
            .map_err(|_| Error::Orchard)?;
    }
    Ok(())
}

fn read_key(bytes: &[u8]) -> Result<PreparedKey> {
    let mut remaining = bytes;
    let key = groth16::VerifyingKey::read(&mut remaining).map_err(|_| Error::Key)?;
    if !remaining.is_empty() {
        return Err(Error::Key);
    }
    Ok(groth16::prepare_verifying_key(&key))
}
fn sapling_spend_key() -> Result<&'static PreparedKey> {
    static KEY: OnceLock<Result<PreparedKey>> = OnceLock::new();
    KEY.get_or_init(|| read_key(include_bytes!("../parameters/sapling-spend.vk")))
        .as_ref()
        .map_err(|_| Error::Key)
}
fn sapling_output_key() -> Result<&'static PreparedKey> {
    static KEY: OnceLock<Result<PreparedKey>> = OnceLock::new();
    KEY.get_or_init(|| read_key(include_bytes!("../parameters/sapling-output.vk")))
        .as_ref()
        .map_err(|_| Error::Key)
}
fn sprout_key() -> Result<&'static PreparedKey> {
    static KEY: OnceLock<Result<PreparedKey>> = OnceLock::new();
    KEY.get_or_init(|| read_key(include_bytes!("../parameters/sprout-groth16.vk")))
        .as_ref()
        .map_err(|_| Error::Key)
}
fn check_proof(key: &PreparedKey, bytes: &[u8; 192], inputs: &[Scalar]) -> Result<()> {
    let proof = groth16::Proof::read(bytes.as_slice()).map_err(|_| Error::Key)?;
    groth16::verify_proof(key, &proof, inputs).map_err(|_| Error::Key)
}
fn orchard_key(version: OrchardCircuitVersion) -> &'static VerifyingKey {
    static ORIGINAL: OnceLock<VerifyingKey> = OnceLock::new();
    static FIXED: OnceLock<VerifyingKey> = OnceLock::new();
    static IRONWOOD: OnceLock<VerifyingKey> = OnceLock::new();
    let key = match version {
        OrchardCircuitVersion::InsecurePreNu6_2 => &ORIGINAL,
        OrchardCircuitVersion::FixedPostNu6_2 => &FIXED,
        OrchardCircuitVersion::PostNu6_3 => &IRONWOOD,
    };
    key.get_or_init(|| VerifyingKey::build(version))
}
