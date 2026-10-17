#![no_main]
use risc0_zkvm::guest::env;
use zutti_protocol::encoding::Canonical;
use zutti_source::query::{MAX_QUERY_WITNESS, QueryWitness};

risc0_zkvm::guest::entry!(main);

fn main() {
    let mut length = [0u8; 4];
    env::read_slice(&mut length);
    let length = u32::from_le_bytes(length) as usize;
    assert!(
        length > 0 && length <= MAX_QUERY_WITNESS,
        "invalid witness length"
    );
    let mut bytes = vec![0; length];
    env::read_slice(&mut bytes);
    let witness = QueryWitness::from_bytes(&bytes).expect("invalid query witness");
    let claim = witness.evaluate().expect("unprovable history query");
    env::commit_slice(&claim.to_bytes().expect("invalid query claim"));
}
