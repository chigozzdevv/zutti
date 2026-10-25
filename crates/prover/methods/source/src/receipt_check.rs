#![no_main]
#[path = "../../receipt.rs"]
mod receipt;
use risc0_zkvm::guest::env;
risc0_zkvm::guest::entry!(main);

fn main() {
    let mut image = [0; 32];
    env::read_slice(&mut image);
    let mut length = [0; 4];
    env::read_slice(&mut length);
    let length = u32::from_le_bytes(length) as usize;
    assert!(
        length > 0 && length <= 65_664,
        "invalid receipt journal length"
    );
    let mut journal = vec![0; length];
    env::read_slice(&mut journal);
    receipt::read_and_verify(image, &journal);
    env::commit_slice(&journal);
}
