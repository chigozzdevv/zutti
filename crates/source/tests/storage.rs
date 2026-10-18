#![cfg(feature = "storage")]
#![allow(clippy::unwrap_used)]
use zcash_protocol::consensus::NetworkType;
use zutti_protocol::{Digest, encoding::Canonical};
use zutti_source::{proof::SourcePolicy, storage::SourceStore};

fn policy(network: NetworkType, start: u32) -> SourcePolicy {
    SourcePolicy {
        program: Digest([21; 32]),
        registry_program: Digest([22; 32]),
        network,
        index_start: start,
        confirmations: 3,
    }
}
fn genesis(network: NetworkType) -> Vec<u8> {
    match network {
        NetworkType::Main => zebra_test::vectors::MAINNET_BLOCKS[&0].to_vec(),
        NetworkType::Test => zebra_test::vectors::TESTNET_BLOCKS[&0].to_vec(),
        _ => unreachable!(),
    }
}
fn blocks(network: NetworkType, first: u32, last: u32) -> Vec<Vec<u8>> {
    let map = match network {
        NetworkType::Main => &*zebra_test::vectors::MAINNET_BLOCKS,
        NetworkType::Test => &*zebra_test::vectors::TESTNET_BLOCKS,
        _ => unreachable!(),
    };
    (first..=last).map(|height| map[&height].to_vec()).collect()
}
fn evidence(store: &mut SourceStore) {
    let state = store
        .candidate()
        .state_snapshot()
        .unwrap()
        .commitment()
        .unwrap();
    store.retain_evidence(state, vec![1]).unwrap();
}
#[test]
fn restored_batches_match_uninterrupted_validation_and_paths() {
    for network in [NetworkType::Main, NetworkType::Test] {
        for start in [0, 5, 20] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("source.sqlite");
            let policy = policy(network, start);
            let mut store = SourceStore::create(&path, genesis(network), policy).unwrap();
            let genesis_state = store
                .candidate()
                .state_snapshot()
                .unwrap()
                .to_bytes()
                .unwrap();
            assert!(store.advance(blocks(network, 1, 1)).is_err());
            drop(store);
            let mut store = SourceStore::open(&path, policy).unwrap();
            assert_eq!(
                store
                    .candidate()
                    .state_snapshot()
                    .unwrap()
                    .to_bytes()
                    .unwrap(),
                genesis_state
            );
            evidence(&mut store);
            let mut baseline = store.candidate().clone();
            baseline.record_transition(blocks(network, 1, 10)).unwrap();
            store.advance(blocks(network, 1, 4)).unwrap();
            let pending = store.pending().unwrap().to_bytes().unwrap();
            drop(store);
            let mut store = SourceStore::open(&path, policy).unwrap();
            assert_eq!(store.pending().unwrap().to_bytes().unwrap(), pending);
            evidence(&mut store);
            drop(store);
            let mut store = SourceStore::open(&path, policy).unwrap();
            assert!(store.pending().is_none());
            assert_eq!(store.evidence(), Some([1].as_slice()));
            store.advance(blocks(network, 5, 10)).unwrap();
            assert_eq!(
                store
                    .candidate()
                    .state_snapshot()
                    .unwrap()
                    .to_bytes()
                    .unwrap(),
                baseline.state_snapshot().unwrap().to_bytes().unwrap()
            );
            assert_eq!(store.pending().unwrap().evaluate().unwrap().height, 10);
            if start <= 10 {
                let key = Digest([37; 32]);
                assert_eq!(
                    store.candidate().prove(key).unwrap().to_bytes().unwrap(),
                    baseline.prove(key).unwrap().to_bytes().unwrap()
                );
            }
        }
    }
}
#[test]
fn invalid_batches_wrong_policies_and_stale_writers_do_not_commit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.sqlite");
    let policy = policy(NetworkType::Main, 0);
    let mut store = SourceStore::create(&path, genesis(NetworkType::Main), policy).unwrap();
    evidence(&mut store);
    let mut stale = SourceStore::open(&path, policy).unwrap();
    let before = store
        .candidate()
        .state_snapshot()
        .unwrap()
        .commitment()
        .unwrap();
    assert!(store.advance(blocks(NetworkType::Main, 2, 2)).is_err());
    assert_eq!(
        store
            .candidate()
            .state_snapshot()
            .unwrap()
            .commitment()
            .unwrap(),
        before
    );
    store.advance(blocks(NetworkType::Main, 1, 2)).unwrap();
    assert!(stale.advance(blocks(NetworkType::Main, 1, 1)).is_err());
    assert_eq!(
        stale
            .candidate()
            .state_snapshot()
            .unwrap()
            .commitment()
            .unwrap(),
        before
    );
    assert!(store.retain_evidence(before, vec![1]).is_err());
    assert!(
        SourceStore::open(
            &path,
            SourcePolicy {
                confirmations: 4,
                ..policy
            }
        )
        .is_err()
    );
    assert!(SourceStore::create(&path, genesis(NetworkType::Main), policy).is_err());
    drop(store);
    drop(stale);
    assert_eq!(
        SourceStore::open(&path, policy)
            .unwrap()
            .candidate()
            .height()
            .unwrap(),
        2
    );
}
#[test]
fn corrupted_authenticated_leaves_are_rejected_on_reopen() {
    for tree in [0, 1, 2, 3] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.sqlite");
        let policy = policy(NetworkType::Main, 0);
        let mut store = SourceStore::create(&path, genesis(NetworkType::Main), policy).unwrap();
        evidence(&mut store);
        store.advance(blocks(NetworkType::Main, 1, 3)).unwrap();
        drop(store);
        let db = rusqlite::Connection::open(&path).unwrap();
        assert!(
            db.execute("DELETE FROM leaves WHERE tree=?1", [tree])
                .unwrap()
                > 0
        );
        drop(db);
        assert!(SourceStore::open(&path, policy).is_err());
    }
}
#[test]
fn interrupted_database_transaction_restores_the_previous_complete_batch() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.sqlite");
    let policy = policy(NetworkType::Main, 0);
    let mut store = SourceStore::create(&path, genesis(NetworkType::Main), policy).unwrap();
    evidence(&mut store);
    drop(store);
    let mut db = rusqlite::Connection::open(&path).unwrap();
    {
        let tx = db.transaction().unwrap();
        tx.execute("DELETE FROM leaves", []).unwrap();
        tx.execute("UPDATE metadata SET state=x'00'", []).unwrap();
    }
    drop(db);
    let store = SourceStore::open(&path, policy).unwrap();
    assert_eq!(store.candidate().height().unwrap(), 0);
    assert!(store.pending().is_none());
}
