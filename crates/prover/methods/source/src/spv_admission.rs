#![no_main]
#[cfg(feature = "boundless")]
#[path = "../../receipt.rs"]
mod receipt;

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
    let journal = witness
        .required_journal()
        .expect("invalid SPV dependency journal");
    #[cfg(feature = "boundless")]
    receipt::read_and_verify(previous.policy().program().0, &journal);
    #[cfg(not(feature = "boundless"))]
    env::verify(previous.policy().program().0, &journal).expect("unverified SPV predecessor");
    let claim = witness
        .evaluate()
        .expect("invalid SPV admission transition");
    env::commit_slice(&claim.to_bytes().expect("invalid SPV admission journal"));
}
