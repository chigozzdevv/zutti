#![allow(clippy::unwrap_used)]

use sha2::{Digest as _, Sha256};
use zcash_protocol::consensus::{MAIN_NETWORK, NetworkType};
use zutti_source::{
    block::ParsedBlock,
    subsidy::MonetaryState,
    transaction::{BlockValue, CandidateLedger},
};

fn raw() -> Vec<u8> {
    hex::decode(include_str!("../../../tests/fixtures/block-1.txt").trim()).unwrap()
}

#[test]
fn real_first_block_pays_the_required_founder_and_slow_start_subsidy() {
    let block = ParsedBlock::parse(&raw(), &MAIN_NETWORK).unwrap();
    let value = CandidateLedger::default().append(&block).unwrap();
    let state = MonetaryState::new(NetworkType::Main).unwrap();
    let next = state.transition(&block, value).unwrap();
    assert_eq!(state.issued(), 0);
    assert_eq!(next.issued(), value.coinbase_value);
    assert_eq!(next.deferred(), 0);
    assert!(
        state
            .transition(
                &block,
                BlockValue {
                    coinbase_value: value.coinbase_value + 1,
                    ..value
                }
            )
            .is_err()
    );
}

#[test]
fn wrong_founder_recipient_fails_even_with_a_consistent_transaction_merkle_root() {
    let mut bytes = raw();
    let block = ParsedBlock::parse(&bytes, &MAIN_NETWORK).unwrap();
    let transaction = &block.transactions()[0];
    let script = &transaction
        .transparent_bundle()
        .unwrap()
        .vout
        .iter()
        .find(|output| output.script_pubkey().0.0.len() == 23)
        .unwrap()
        .script_pubkey()
        .0
        .0;
    let position = bytes
        .windows(script.len())
        .position(|window| window == script)
        .unwrap();
    bytes[position + 2] ^= 1;
    let mut tx_bytes = Vec::new();
    transaction.write(&mut tx_bytes).unwrap();
    let offset = bytes.len() - tx_bytes.len();
    let txid = Sha256::digest(Sha256::digest(&bytes[offset..]));
    bytes[36..68].copy_from_slice(&txid);
    let changed = ParsedBlock::parse(&bytes, &MAIN_NETWORK).unwrap();
    let value = CandidateLedger::default().append(&changed).unwrap();
    assert!(
        MonetaryState::new(NetworkType::Main)
            .unwrap()
            .transition(&changed, value)
            .is_err()
    );
}
