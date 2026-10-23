#![no_main]

use risc0_zkvm::guest::env;
use sha2::{Digest, Sha256};
use zcash_protocol::consensus::{MAIN_NETWORK, TEST_NETWORK};
use zutti_protocol::encoding::Reader;
use zutti_source::{block::ParsedBlock, trees::CandidateTrees};

risc0_zkvm::guest::entry!(main);

fn main() {
    let mut length = [0; 4];
    env::read_slice(&mut length);
    let length = u32::from_le_bytes(length) as usize;
    assert!((1..=2_000_064).contains(&length));
    let mut bytes = vec![0; length];
    env::read_slice(&mut bytes);
    let mut reader = Reader::new(&bytes);
    assert_eq!(reader.fixed::<8>().expect("profile magic"), *b"ZUTTIVAL");
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
    assert!(index < block.transactions().len());
    let mut trees = CandidateTrees::default();
    let mut digest = Sha256::new();
    digest.update(Sha256::digest(&bytes));
    for _ in 0..repetitions {
        trees
            .append(block.commitment_updates())
            .expect("profile tree update");
        let roots = trees.roots();
        for root in [
            trees.sprout_root(),
            roots.sapling,
            roots.orchard,
            roots.ironwood,
        ] {
            digest.update(root.0);
        }
    }
    env::commit_slice(&digest.finalize());
}
