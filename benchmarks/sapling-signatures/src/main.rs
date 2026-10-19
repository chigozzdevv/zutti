#![no_main]

use risc0_zkvm::guest::env;
use sha2::{Digest, Sha256};

#[cfg(feature = "accelerated")]
#[allow(dead_code)]
#[path = "../../../crates/source/src/authorization/signatures.rs"]
mod checker;

risc0_zkvm::guest::entry!(main);

fn main() {
    let mut count = [0_u8];
    env::read_slice(&mut count);
    assert!((1..=64).contains(&count[0]), "invalid signature count");
    let mut body = vec![0; usize::from(count[0]) * 128];
    env::read_slice(&mut body);
    for record in body.chunks_exact(128) {
        let key: [u8; 32] = record[..32].try_into().expect("public key");
        let signature: [u8; 64] = record[64..].try_into().expect("signature");
        #[cfg(feature = "accelerated")]
        let valid = checker::binding(key, signature, &record[32..64]);
        #[cfg(not(feature = "accelerated"))]
        let valid =
            redjubjub::VerificationKey::<redjubjub::Binding>::try_from(key).is_ok_and(|key| {
                key.verify(&record[32..64], &redjubjub::Signature::from(signature))
                    .is_ok()
            });
        assert!(valid, "invalid signature");
    }
    let mut digest = Sha256::new();
    digest.update(count);
    digest.update(body);
    env::commit_slice(&digest.finalize());
}
