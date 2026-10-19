#![no_main]
use bls12_381::Scalar;
use risc0_zkvm::guest::env;
use sha2::{Digest, Sha256};
use zutti_protocol::encoding::Reader;

#[derive(Debug)]
enum Error {
    Key,
}
type Result<T> = core::result::Result<T, Error>;

#[path = "../../../crates/source/src/authorization/groth.rs"]
mod groth;

risc0_zkvm::guest::entry!(main);

fn main() {
    let mut length = [0; 4];
    env::read_slice(&mut length);
    let length = u32::from_le_bytes(length) as usize;
    assert!(
        (1..=65_536).contains(&length),
        "invalid profile input length"
    );
    let mut bytes = vec![0; length];
    env::read_slice(&mut bytes);
    let mut reader = Reader::new(&bytes);
    assert_eq!(reader.fixed::<8>().expect("profile magic"), *b"ZUTTIG16");
    let kind = reader.u8().expect("profile kind");
    let count = reader.u16().expect("profile count");
    assert!((1..=64).contains(&count), "invalid profile count");
    let (key, arity) = match kind {
        0 => (
            include_bytes!("../../../crates/source/parameters/sapling-output.vk").as_slice(),
            5,
        ),
        1 => (
            include_bytes!("../../../crates/source/parameters/sapling-spend.vk").as_slice(),
            7,
        ),
        2 => (
            include_bytes!("../../../crates/source/parameters/sprout-groth16.vk").as_slice(),
            9,
        ),
        _ => panic!("unsupported profile kind"),
    };
    let key = groth::read_key(key).expect("invalid embedded key");
    for _ in 0..count {
        let proof = reader.fixed::<192>().expect("profile proof");
        let inputs = (0..arity)
            .map(|_| {
                let bytes = reader.fixed::<32>().expect("profile input");
                Option::<Scalar>::from(Scalar::from_bytes(&bytes))
                    .expect("noncanonical profile input")
            })
            .collect::<Vec<_>>();
        groth::check_proof(&key, &proof, &inputs).expect("invalid profile proof");
    }
    reader.finish().expect("trailing profile bytes");
    env::commit_slice(&Sha256::digest(&bytes));
}
