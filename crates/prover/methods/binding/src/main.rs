#![no_main]

use risc0_zkvm::guest::env;
use zutti_protocol::encoding::Canonical;
use zutti_wallet::binding::{MAX_BINDING_WITNESS, PaymentBindingWitness};

risc0_zkvm::guest::entry!(main);

fn main() {
    let mut length = [0; 4];
    env::read_slice(&mut length);
    let length = u32::from_le_bytes(length) as usize;
    assert!(
        length > 0 && length <= MAX_BINDING_WITNESS,
        "invalid binding input length"
    );
    let mut bytes = vec![0; length];
    env::read_slice(&mut bytes);
    let witness =
        PaymentBindingWitness::from_bytes(&bytes).expect("invalid payment binding witness");
    let claim = witness.evaluate().expect("invalid payment window binding");
    env::commit_slice(&claim.to_bytes().expect("invalid payment binding journal"));
}
