use k256::ecdsa::{Signature, VerifyingKey, signature::hazmat::PrehashVerifier};
use zcash_script::{
    interpreter::{CallbackTransactionSignatureChecker, SignatureChecker},
    script::Code,
    signature::Decoded,
};

#[derive(Clone, Copy)]
pub(super) struct Checker<'a>(pub(super) CallbackTransactionSignatureChecker<'a>);

impl core::fmt::Debug for Checker<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("Checker")
            .field("lock_time", &self.0.lock_time)
            .field("is_final", &self.0.is_final)
            .finish_non_exhaustive()
    }
}

impl SignatureChecker for Checker<'_> {
    fn check_sig(&self, signature: &Decoded, public: &[u8], code: &Code) -> bool {
        if public.is_empty() {
            return false;
        }
        let Some(digest) = (self.0.sighash)(code, signature.sighash_type()) else {
            return false;
        };
        let Ok(public) = secp256k1::PublicKey::from_slice(public) else {
            return false;
        };
        let mut normalized = *signature.sig();
        normalized.normalize_s();
        let Ok(key) = VerifyingKey::from_sec1_bytes(&public.serialize()) else {
            return false;
        };
        let Ok(signature) = Signature::from_slice(&normalized.serialize_compact()) else {
            return false;
        };
        key.verify_prehash(&digest, &signature).is_ok()
    }

    fn check_lock_time(&self, lock_time: i64) -> bool {
        self.0.check_lock_time(lock_time)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use secp256k1::{Message, SecretKey, ecdsa};
    use zcash_script::signature::Validity;

    fn decoded(signature: &ecdsa::Signature) -> Decoded {
        let mut encoded = signature.serialize_der().to_vec();
        encoded.push(1);
        match Decoded::from_bytes(&encoded, false, false) {
            Validity::Valid(value) => value,
            _ => panic!("invalid generated signature"),
        }
    }

    #[test]
    fn signatures_match_reference_for_high_s_and_all_public_key_encodings() {
        for seed in 1..=8 {
            let secret = SecretKey::from_secret_bytes([seed; 32]).unwrap();
            let public = secret.public_key();
            let mut hybrid = public.serialize_uncompressed();
            hybrid[0] = 6 | (hybrid[64] & 1);
            for digest in [[0; 32], [255; 32], [seed; 32]] {
                let low = ecdsa::sign(Message::from_digest(digest), &secret);
                let mut compact = low.serialize_compact();
                let scalar = SecretKey::from_secret_bytes(compact[32..].try_into().unwrap())
                    .unwrap()
                    .negate()
                    .to_secret_bytes();
                compact[32..].copy_from_slice(&scalar);
                let high = ecdsa::Signature::from_compact(&compact).unwrap();
                assert_ne!(low, high);
                for signature in [low, high] {
                    let signature = decoded(&signature);
                    let hash = |_: &Code, _: &zcash_script::signature::HashType| Some(digest);
                    let reference = CallbackTransactionSignatureChecker {
                        sighash: &hash,
                        lock_time: 123,
                        is_final: false,
                    };
                    let checker = Checker(reference);
                    for key in [
                        public.serialize().to_vec(),
                        public.serialize_uncompressed().to_vec(),
                        hybrid.to_vec(),
                    ] {
                        assert!(reference.check_sig(&signature, &key, &Code(vec![])));
                        assert!(checker.check_sig(&signature, &key, &Code(vec![])));
                    }
                }
            }
        }
    }

    #[test]
    fn changed_digests_malformed_keys_and_zero_scalars_fail() {
        let secret = SecretKey::from_secret_bytes([7; 32]).unwrap();
        let signature = decoded(&ecdsa::sign(Message::from_digest([1; 32]), &secret));
        let hash = |_: &Code, _: &zcash_script::signature::HashType| Some([2; 32]);
        let reference = CallbackTransactionSignatureChecker {
            sighash: &hash,
            lock_time: 123,
            is_final: false,
        };
        let public = secret.public_key().serialize();
        assert!(!reference.check_sig(&signature, &public, &Code(vec![])));
        assert!(!Checker(reference).check_sig(&signature, &public, &Code(vec![])));
        let correct = |_: &Code, _: &zcash_script::signature::HashType| Some([1; 32]);
        let reference = CallbackTransactionSignatureChecker {
            sighash: &correct,
            ..reference
        };
        let mut wrong_hybrid = secret.public_key().serialize_uncompressed();
        wrong_hybrid[0] = 6 | ((wrong_hybrid[64] & 1) ^ 1);
        for public in [vec![], vec![0; 65], vec![255; 33], wrong_hybrid.to_vec()] {
            assert!(!reference.check_sig(&signature, &public, &Code(vec![])));
            assert!(!Checker(reference).check_sig(&signature, &public, &Code(vec![])));
        }
        let zero = decoded(&ecdsa::Signature::from_compact(&[0; 64]).unwrap());
        assert!(!reference.check_sig(&zero, &public, &Code(vec![])));
        assert!(!Checker(reference).check_sig(&zero, &public, &Code(vec![])));
    }
}
