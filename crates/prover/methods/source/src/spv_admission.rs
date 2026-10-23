#![no_main]

use risc0_zkvm::guest::env;
use zutti_protocol::encoding::Canonical;
use zutti_source::spv::{AdmissionWitness, MAX_WITNESS};

risc0_zkvm::guest::entry!(main);

fn main() {
    let mut length = [0; 4];
    env::read_slice(&mut length);
    let length = u32::from_le_bytes(length) as usize;
    assert!(
        length > 0 && length <= MAX_WITNESS,
        "invalid SPV admission input length"
    );
    let mut bytes = vec![0; length];
    env::read_slice(&mut bytes);
    let witness = AdmissionWitness::from_bytes(&bytes).expect("invalid SPV admission witness");
    let previous = witness.predecessor();
    env::verify(
        previous.policy().program().0,
        &previous.to_bytes().expect("invalid SPV predecessor"),
    )
    .expect("unverified SPV predecessor");
    let claim = witness
        .evaluate()
        .expect("invalid SPV admission transition");
    env::commit_slice(&claim.to_bytes().expect("invalid SPV admission journal"));
}
