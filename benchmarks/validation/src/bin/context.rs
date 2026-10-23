#![no_main]

use risc0_zkvm::guest::env;
use sha2::{Digest, Sha256};
use zcash_protocol::consensus::{MAIN_NETWORK, TEST_NETWORK};
use zutti_protocol::encoding::Reader;
use zutti_source::{block::ParsedBlock, transaction::check_context};

risc0_zkvm::guest::entry!(main);

fn main() {
    let mut length = [0; 4];
    env::read_slice(&mut length);
    let length = u32::from_le_bytes(length) as usize;
    assert!((1..=2_000_064).contains(&length));
    let mut bytes = vec![0; length];
    env::read_slice(&mut bytes);
    let mut reader = Reader::new(&bytes);
    assert_eq!(reader.fixed::<8>().expect("profile magic"), *b"ZUTTICX1");
    let network = reader.u8().expect("profile network");
    let index = reader.u32().expect("profile transaction") as usize;
    let repetitions = reader.u16().expect("profile repetitions");
    assert!((1..=16).contains(&repetitions));
    let raw = reader.bytes(2_000_000).expect("profile block");
    reader.finish().expect("trailing profile bytes");
    let block = match network {
        0 => ParsedBlock::parse(raw, &MAIN_NETWORK),
        1 => ParsedBlock::parse(raw, &TEST_NETWORK),
        _ => panic!("unsupported profile network"),
    }
    .expect("invalid profile block");
    let tx = block
        .transactions()
        .get(index)
        .expect("profile transaction");
    let coinbase = tx
        .transparent_bundle()
        .is_some_and(|bundle| bundle.is_coinbase());
    for _ in 0..repetitions {
        check_context(
            tx,
            block.branch(),
            block.claimed_height(),
            block.header().time,
            coinbase,
        )
        .expect("invalid transaction context");
    }
    env::commit_slice(&Sha256::digest(&bytes));
}
