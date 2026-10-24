#![no_main]
use risc0_zkvm::guest::env;
use zutti_protocol::encoding::Canonical;
use zutti_wallet::spv_pool::{MAX_SPV_POOL_WITNESS, SpvPoolWitness};
risc0_zkvm::guest::entry!(main);
fn main() {
    let mut length = [0; 4];
    env::read_slice(&mut length);
    let length = u32::from_le_bytes(length) as usize;
    assert!(
        length > 0 && length <= MAX_SPV_POOL_WITNESS,
        "invalid SPV pool input length"
    );
    let mut bytes = vec![0; length];
    env::read_slice(&mut bytes);
    let witness = SpvPoolWitness::from_bytes(&bytes).expect("invalid SPV pool witness");
    for (image, journal) in witness.assumptions().expect("invalid SPV pool operation") {
        env::verify(image.0, &journal).expect("unverified SPV pool dependency");
    }
    let claim = witness.evaluate().expect("invalid SPV pool operation");
    env::commit_slice(&claim.to_bytes().expect("invalid SPV pool journal"));
}
