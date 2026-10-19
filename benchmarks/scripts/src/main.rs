#![no_main]

use risc0_zkvm::guest::env;
use sha2::{Digest, Sha256};
use zcash_script::{
    interpreter::{CallbackTransactionSignatureChecker, SignatureChecker},
    script::Code,
    signature::{Decoded, HashType, Validity},
};
use zutti_protocol::encoding::Reader;

#[cfg(feature = "accelerated-scripts")]
#[path = "../../../crates/source/src/transaction/signatures.rs"]
mod checker;

risc0_zkvm::guest::entry!(main);

fn main() {
    let mut length = [0; 4];
    env::read_slice(&mut length);
    let length = u32::from_le_bytes(length) as usize;
    assert!(
        (1..=16_384).contains(&length),
        "invalid profile input length"
    );
    let mut bytes = vec![0; length];
    env::read_slice(&mut bytes);
    let mut reader = Reader::new(&bytes);
    assert_eq!(reader.fixed::<8>().expect("profile magic"), *b"ZUTTISIG");
    let count = reader.u16().expect("signature count");
    assert!((1..=64).contains(&count), "invalid signature count");
    for _ in 0..count {
        let digest = reader.fixed::<32>().expect("signature digest");
        let public = reader.bytes(65).expect("public key");
        let signature =
            match Decoded::from_bytes(reader.bytes(74).expect("signature bytes"), false, false) {
                Validity::Valid(value) => value,
                _ => panic!("invalid signature encoding"),
            };
        let hash = |_: &Code, _: &HashType| Some(digest);
        let verifier = CallbackTransactionSignatureChecker {
            sighash: &hash,
            lock_time: 123,
            is_final: false,
        };
        #[cfg(feature = "accelerated-scripts")]
        let verifier = checker::Checker(verifier);
        assert!(
            verifier.check_sig(&signature, public, &Code(vec![])),
            "invalid signature"
        );
    }
    reader.finish().expect("trailing profile input");
    env::commit_slice(&Sha256::digest(&bytes));
}
