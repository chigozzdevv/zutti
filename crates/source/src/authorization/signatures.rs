use redjubjub_legacy::{Binding, SigType, Signature, SpendAuth, VerificationKey};

pub(super) fn spend(key: [u8; 32], signature: [u8; 64], message: &[u8]) -> bool {
    check::<SpendAuth>(key, signature, message)
}

pub(super) fn binding(key: [u8; 32], signature: [u8; 64], message: &[u8]) -> bool {
    check::<Binding>(key, signature, message)
}

fn check<T: SigType>(key: [u8; 32], signature: [u8; 64], message: &[u8]) -> bool {
    VerificationKey::<T>::try_from(key)
        .is_ok_and(|key| key.verify(message, &Signature::from(signature)).is_ok())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use group::ff::PrimeField;
    use zcash_protocol::consensus::MAIN_NETWORK;
    use zcash_script::script::Code;

    fn reference<T: redjubjub::SigType>(
        key: [u8; 32],
        signature: [u8; 64],
        message: &[u8],
    ) -> bool {
        redjubjub::VerificationKey::<T>::try_from(key).is_ok_and(|key| {
            key.verify(message, &redjubjub::Signature::from(signature))
                .is_ok()
        })
    }

    #[test]
    fn historical_spend_and_binding_signatures_match_the_current_verifier() {
        let mut spends = 0;
        let mut bindings = 0;
        for text in [
            include_str!("../../../../tests/fixtures/block-1687106.txt"),
            include_str!("../../../../tests/fixtures/block-1687107.txt"),
            include_str!("../../../../tests/fixtures/block-1687108.txt"),
        ] {
            let raw = hex::decode(text.trim()).unwrap();
            let block = crate::block::ParsedBlock::parse(&raw, &MAIN_NETWORK).unwrap();
            for tx in block.transactions() {
                if tx
                    .transparent_bundle()
                    .is_some_and(|b| !b.is_coinbase() && !b.vin.is_empty())
                {
                    continue;
                }
                let Some(bundle) = tx.sapling_bundle() else {
                    continue;
                };
                let message =
                    crate::transaction::input_signature_hash(tx, &[], None, &Code(vec![]), 1)
                        .unwrap();
                let mut changed = message;
                changed[0] ^= 1;
                let mut sum = sapling::value::CommitmentSum::zero();
                for spend in bundle.shielded_spends() {
                    let key: [u8; 32] = (*spend.rk()).into();
                    let signature: [u8; 64] = (*spend.spend_auth_sig()).into();
                    assert!(reference::<redjubjub::SpendAuth>(key, signature, &message));
                    assert!(super::spend(key, signature, &message));
                    assert!(!super::spend(key, signature, &changed));
                    sum += spend.cv();
                    spends += 1;
                }
                for output in bundle.shielded_outputs() {
                    sum -= output.cv();
                }
                let key = sum.into_bvk(*bundle.value_balance()).into();
                let signature = bundle.authorization().binding_sig.into();
                assert!(reference::<redjubjub::Binding>(key, signature, &message));
                assert!(super::binding(key, signature, &message));
                assert!(!super::binding(key, signature, &changed));
                bindings += 1;
            }
        }
        assert!(spends > 0 && bindings > 0);
    }

    #[test]
    fn encoding_scalar_and_cofactor_boundaries_preserve_reference_acceptance() {
        let mut identity = [0; 32];
        identity[0] = 1;
        let mut noncanonical = [255; 32];
        noncanonical[31] = 127;
        let mut negative_zero = identity;
        negative_zero[31] = 128;
        let little_endian = |value: &str| -> [u8; 32] {
            let mut bytes = hex::decode(value.trim_start_matches("0x")).unwrap();
            bytes.reverse();
            bytes.try_into().unwrap()
        };
        let field_modulus = little_endian(bls12_381::Scalar::MODULUS);
        let scalar_modulus = little_endian(jubjub::Fr::MODULUS);
        let mut below_modulus = scalar_modulus;
        below_modulus[0] -= 1;
        let mut above_modulus = scalar_modulus;
        above_modulus[0] += 1;
        let points = [
            [0; 32],
            [255; 32],
            noncanonical,
            identity,
            negative_zero,
            field_modulus,
        ];
        let scalars = [
            [0; 32],
            [255; 32],
            noncanonical,
            identity,
            below_modulus,
            scalar_modulus,
            above_modulus,
        ];
        for key in points {
            for r in points {
                for s in scalars {
                    let mut signature = [0; 64];
                    signature[..32].copy_from_slice(&r);
                    signature[32..].copy_from_slice(&s);
                    for message in [&[][..], &[0; 32][..], &[255; 32][..]] {
                        assert_eq!(
                            binding(key, signature, message),
                            reference::<redjubjub::Binding>(key, signature, message)
                        );
                        assert_eq!(
                            spend(key, signature, message),
                            reference::<redjubjub::SpendAuth>(key, signature, message)
                        );
                    }
                }
            }
        }
    }
}
