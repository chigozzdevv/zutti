#![no_main]

use risc0_zkvm::guest::env;
use sha2::{Digest, Sha256};
use zcash_protocol::consensus::TEST_NETWORK;
use zutti_protocol::encoding::{Reader, Writer};
use zutti_source::{authorization, block::ParsedBlock};

risc0_zkvm::guest::entry!(main);

fn main() {
    let mut length = [0; 4];
    env::read_slice(&mut length);
    let length = u32::from_le_bytes(length) as usize;
    assert!((1..=32_001_024).contains(&length));
    let mut bytes = vec![0; length];
    env::read_slice(&mut bytes);
    let mut reader = Reader::new(&bytes);
    assert_eq!(reader.fixed::<8>().expect("profile magic"), *b"ZUTTIVB1");
    let count = reader.u8().expect("profile records");
    assert!((1..=16).contains(&count));
    let mut report = Writer::default();
    report.fixed(&Sha256::digest(&bytes));
    report.u8(count);
    for record in 0..count {
        let index = reader.u32().expect("profile transaction") as usize;
        let repetitions = reader.u8().expect("profile repetitions");
        assert!((2..=16).contains(&repetitions));
        let raw = reader.bytes(2_000_000).expect("profile block");
        let started = env::cycle_count();
        let block = ParsedBlock::parse(raw, &TEST_NETWORK).expect("invalid profile block");
        report.u64(env::cycle_count() - started);
        let tx = block
            .transactions()
            .get(index)
            .expect("profile transaction");
        assert!(
            tx.transparent_bundle()
                .is_none_or(|bundle| bundle.is_coinbase() || bundle.vin.is_empty())
        );
        report.u8(repetitions);
        for repetition in 0..repetitions {
            let started = env::cycle_count();
            authorization::verify(tx, &[]).expect("invalid transaction authorization");
            let cycles = env::cycle_count() - started;
            report.u64(cycles);
            env::log(&format!(
                "record={record} repetition={repetition} verification_cycles={cycles}"
            ));
        }
    }
    reader.finish().expect("trailing profile bytes");
    env::commit_slice(&report.finish());
}
