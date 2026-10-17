#![no_main]
use risc0_zkvm::guest::env;
use zutti_protocol::encoding::Canonical;
use zutti_source::{proof::SourceWitness, transition::MAX_WITNESS};

mod seal;

risc0_zkvm::guest::entry!(main);

fn main() {
    let mut length = [0; 4];
    env::read_slice(&mut length);
    let length = u32::from_le_bytes(length) as usize;
    assert!(
        length > 0 && length <= MAX_WITNESS,
        "invalid source witness length"
    );
    let mut bytes = vec![0; length];
    env::read_slice(&mut bytes);
    let witness = SourceWitness::from_bytes(&bytes).expect("invalid source witness");
    let claim = witness.evaluate().expect("invalid source transition");
    if let Some(previous) = witness.assumption() {
        assert_eq!(
            previous.program, witness.policy.program,
            "source program mismatch"
        );
        seal::verify(
            previous.program.0,
            previous.to_bytes().expect("invalid predecessor"),
        );
    }
    env::commit_slice(&claim.to_bytes().expect("invalid source claim"));
}
