#![no_main]
use risc0_zkvm::guest::env;
use zutti_protocol::encoding::Canonical;
use zutti_wallet::pool::{MAX_POOL_WITNESS, PoolWitness};

risc0_zkvm::guest::entry!(main);

fn main() {
    let mut length = [0; 4];
    env::read_slice(&mut length);
    let length = u32::from_le_bytes(length) as usize;
    assert!(
        length > 0 && length <= MAX_POOL_WITNESS,
        "invalid pool witness length"
    );
    let mut bytes = vec![0; length];
    env::read_slice(&mut bytes);
    let witness = PoolWitness::from_bytes(&bytes).expect("invalid pool witness");
    let claim = witness.evaluate().expect("invalid private pool operation");
    env::commit_slice(&claim.to_bytes().expect("invalid pool claim"));
}
