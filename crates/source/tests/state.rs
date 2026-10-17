#![allow(clippy::unwrap_used)]

use zcash_protocol::consensus::{MAIN_NETWORK, NetworkType};
use zutti_protocol::Digest;
use zutti_source::{
    Observation,
    block::ParsedBlock,
    bootstrap::GenesisBlock,
    state::{SourceCandidate, TransitionError},
};

fn genesis() -> GenesisBlock {
    GenesisBlock::parse(
        &hex::decode(include_str!("../../../tests/fixtures/block-0.txt").trim()).unwrap(),
        NetworkType::Main,
    )
    .unwrap()
}

fn first_bytes() -> Vec<u8> {
    hex::decode(include_str!("../../../tests/fixtures/block-1.txt").trim()).unwrap()
}

#[test]
fn source_updates_bind_real_headers_trees_and_transaction_history() {
    let genesis = genesis();
    let mut source = SourceCandidate::from_genesis(&genesis, Digest([1; 32]), 0).unwrap();
    let first = ParsedBlock::parse(&first_bytes(), &MAIN_NETWORK).unwrap();
    source.append(&first).unwrap();
    assert_eq!(source.height().unwrap(), 1);
    assert_eq!(source.tip().unwrap(), first.hash());
    assert!(source.cumulative_work().unwrap() > [0; 32]);
    assert_eq!(source.history_root().unwrap(), None);
    let snapshot = source.index_snapshot().unwrap();
    assert_eq!(snapshot.context.genesis(), genesis.block().hash());
    assert_eq!(snapshot.height, source.height().unwrap());
    for txid in first.txids() {
        assert_eq!(
            source
                .prove(*txid)
                .unwrap()
                .verify(snapshot, *txid)
                .unwrap(),
            Observation::Present(1)
        );
    }
    assert_eq!(
        source
            .prove(Digest([9; 32]))
            .unwrap()
            .verify(snapshot, Digest([9; 32]))
            .unwrap(),
        Observation::Absent
    );
}

#[test]
fn invalid_work_and_duplicate_blocks_do_not_partially_advance_source_state() {
    let mut source = SourceCandidate::from_genesis(&genesis(), Digest([1; 32]), 0).unwrap();
    let initial = source.index_snapshot().unwrap();
    let initial_work = source.cumulative_work().unwrap();
    let initial_roots = source.tree_roots();
    let initial_tip = source.tip().unwrap();
    let mut raw = first_bytes();
    raw[108] ^= 1;
    let invalid = ParsedBlock::parse(&raw, &MAIN_NETWORK).unwrap();
    assert!(source.append(&invalid).is_err());
    assert_eq!(source.index_snapshot().unwrap(), initial);
    assert_eq!(source.cumulative_work().unwrap(), initial_work);
    assert_eq!(source.tree_roots().sapling, initial_roots.sapling);
    assert_eq!(source.tree_roots().orchard, initial_roots.orchard);
    assert_eq!(source.tree_roots().ironwood, initial_roots.ironwood);
    assert_eq!(source.tip().unwrap(), initial_tip);
    let first = ParsedBlock::parse(&first_bytes(), &MAIN_NETWORK).unwrap();
    source.append(&first).unwrap();
    let after = source.index_snapshot().unwrap();
    assert!(source.append(&first).is_err());
    assert_eq!(source.index_snapshot().unwrap(), after);
    assert_eq!(source.tip().unwrap(), first.hash());
}

#[test]
fn future_index_start_never_exposes_an_unprocessed_index_as_ready() {
    let mut source = SourceCandidate::from_genesis(&genesis(), Digest([1; 32]), 100).unwrap();
    assert!(matches!(
        source.index_snapshot(),
        Err(TransitionError::IndexUnavailable)
    ));
    source
        .append(&ParsedBlock::parse(&first_bytes(), &MAIN_NETWORK).unwrap())
        .unwrap();
    assert_eq!(source.height().unwrap(), 1);
    assert!(matches!(
        source.index_snapshot(),
        Err(TransitionError::IndexUnavailable)
    ));
    assert!(source.prove(Digest([9; 32])).is_err());
}
