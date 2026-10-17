#![no_main]
use risc0_zkvm::guest::env;
use zutti_protocol::encoding::Canonical;
use zutti_wallet::preparation::{MAX_PREPARATION_WITNESS, PreparationWitness};

risc0_zkvm::guest::entry!(main);

fn main() {
    let mut length = [0u8; 4];
    env::read_slice(&mut length);
    let length = u32::from_le_bytes(length) as usize;
    assert!(
        length > 0 && length <= MAX_PREPARATION_WITNESS,
        "invalid witness length"
    );
    let mut bytes = vec![0; length];
    env::read_slice(&mut bytes);
    let witness = PreparationWitness::from_bytes(&bytes).expect("invalid preparation witness");
    let claim = witness.evaluate().expect("invalid prepared payment");
    env::commit_slice(&claim.to_bytes().expect("invalid preparation claim"));
}
