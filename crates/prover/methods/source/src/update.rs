#![no_main]
use risc0_zkvm::guest::env;
use zutti_protocol::encoding::Canonical;
use zutti_source::update::SourceUpdateWitness;

mod seal;

risc0_zkvm::guest::entry!(main);

fn main() {
    let mut length = [0; 4];
    env::read_slice(&mut length);
    let length = u32::from_le_bytes(length) as usize;
    assert!(
        length > 0 && length <= 65_536,
        "invalid source update length"
    );
    let mut bytes = vec![0; length];
    env::read_slice(&mut bytes);
    let witness = SourceUpdateWitness::from_bytes(&bytes).expect("invalid source update witness");
    let claim = witness.evaluate().expect("invalid source fork update");
    seal::verify(
        witness.policy.program.0,
        witness.chain.to_bytes().expect("invalid source chain"),
    );
    if let Some(previous) = &witness.previous {
        seal::verify(
            witness.policy.registry_program.0,
            previous.claim.to_bytes().expect("invalid previous update"),
        );
    }
    env::commit_slice(&claim.to_bytes().expect("invalid source update claim"));
}
