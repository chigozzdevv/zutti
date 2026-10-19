#![no_main]

use risc0_zkvm::guest::env;

#[cfg(feature = "windowed-trees")]
#[path = "../../../crates/source/src/trees/sapling_hash.rs"]
mod sapling_hash;

risc0_zkvm::guest::entry!(main);

fn main() {
    let mut count = [0; 2];
    env::read_slice(&mut count);
    let count = u16::from_le_bytes(count);
    assert!((1..=1024).contains(&count), "invalid tree hash count");
    let mut left = [0; 32];
    let mut right = [0; 32];
    env::read_slice(&mut left);
    env::read_slice(&mut right);
    for index in 0..usize::from(count) {
        #[cfg(feature = "windowed-trees")]
        {
            left = sapling_hash::merkle_hash(index % 32, &left, &right);
        }
        #[cfg(not(feature = "windowed-trees"))]
        {
            left = sapling::merkle_hash(index % 32, &left, &right);
        }
    }
    env::commit_slice(&left);
}
