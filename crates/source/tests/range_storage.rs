#![cfg(feature = "storage")]
#![allow(clippy::unwrap_used)]
use zcash_protocol::consensus::NetworkType;
use zutti_protocol::{Digest, encoding::Canonical};
use zutti_source::{
    proof::SourcePolicy,
    storage::{RangeLimits, RangeStore, SourceStore},
};

fn policy() -> SourcePolicy {
    SourcePolicy {
        program: Digest([21; 32]),
        registry_program: Digest([22; 32]),
        network: NetworkType::Test,
        index_start: 0,
        confirmations: 3,
    }
}
fn genesis() -> Vec<u8> {
    zebra_test::vectors::TESTNET_BLOCKS[&0].to_vec()
}
fn blocks(first: u32, last: u32) -> Vec<Vec<u8>> {
    (first..=last)
        .map(|h| zebra_test::vectors::TESTNET_BLOCKS[&h].to_vec())
        .collect()
}
fn queue(path: &std::path::Path, limits: RangeLimits) -> RangeStore {
    RangeStore::create(path, genesis(), policy(), limits).unwrap()
}
fn retain(store: &mut RangeStore, id: u64) {
    store
        .retain_range(id, &store.batch(id).unwrap().claim.clone(), vec![1])
        .unwrap();
}
#[test]
fn independent_pending_ranges_restart_without_any_prefix_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ranges.sqlite");
    let limits = RangeLimits::default();
    let mut store = queue(&path, limits);
    let mut baseline = store.candidate().clone();
    baseline.record_transition(blocks(1, 4)).unwrap();
    let a = store.advance(blocks(1, 2)).unwrap();
    let b = store.advance(blocks(3, 4)).unwrap();
    let witness = store
        .batch(b)
        .unwrap()
        .witness
        .as_ref()
        .unwrap()
        .to_bytes()
        .unwrap();
    assert!(store.evidence().is_none());
    assert!(SourceStore::open(&path, policy()).is_err());
    drop(store);
    let mut store = RangeStore::open(&path, policy(), limits).unwrap();
    assert_eq!(store.batches().len(), 2);
    assert_eq!(
        store
            .candidate()
            .state_snapshot()
            .unwrap()
            .to_bytes()
            .unwrap(),
        baseline.state_snapshot().unwrap().to_bytes().unwrap()
    );
    assert_eq!(
        store
            .batch(b)
            .unwrap()
            .witness
            .as_ref()
            .unwrap()
            .to_bytes()
            .unwrap(),
        witness
    );
    retain(&mut store, b);
    drop(store);
    let mut store = RangeStore::open(&path, policy(), limits).unwrap();
    assert!(store.batch(b).unwrap().witness.is_none());
    assert!(store.batch(a).unwrap().witness.is_some());
    retain(&mut store, a);
    let claim = store.aggregation(&[a, b]).unwrap();
    store.retain_aggregate(&[a, b], &claim, vec![2]).unwrap();
    assert_eq!(store.batches().len(), 1);
    drop(store);
    let mut store = RangeStore::open(&path, policy(), limits).unwrap();
    store.retain_prefix(&claim.end, vec![3]).unwrap();
    assert!(store.batches().is_empty());
    drop(store);
    let mut store = RangeStore::open(&path, policy(), limits).unwrap();
    assert_eq!(store.base().height, 4);
    assert_eq!(store.evidence(), Some([3].as_slice()));
    assert!(store.advance(blocks(5, 5)).unwrap() > b);
}
#[test]
fn bounds_and_failed_native_transitions_leave_the_frontier_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ranges.sqlite");
    let limits = RangeLimits {
        batches: 2,
        witness_bytes: 128 * 1024 * 1024,
    };
    let mut store = queue(&path, limits);
    let before = store
        .candidate()
        .state_snapshot()
        .unwrap()
        .commitment()
        .unwrap();
    assert!(store.advance(blocks(2, 2)).is_err());
    assert_eq!(
        store
            .candidate()
            .state_snapshot()
            .unwrap()
            .commitment()
            .unwrap(),
        before
    );
    let first = store.advance(blocks(1, 1)).unwrap();
    let second = store.advance(blocks(2, 2)).unwrap();
    assert!(store.advance(blocks(3, 3)).is_err());
    retain(&mut store, first);
    retain(&mut store, second);
    assert!(store.advance(blocks(3, 3)).is_err());
    let aggregate = store.aggregation(&[first, second]).unwrap();
    store
        .retain_aggregate(&[first, second], &aggregate, vec![4])
        .unwrap();
    store.advance(blocks(3, 3)).unwrap();
    assert_eq!(store.candidate().height().unwrap(), 3);
    let tiny = dir.path().join("tiny.sqlite");
    let mut store = queue(
        &tiny,
        RangeLimits {
            batches: 2,
            witness_bytes: 1,
        },
    );
    assert!(store.advance(blocks(1, 1)).is_err());
    assert_eq!(store.candidate().height().unwrap(), 0);
}
#[test]
fn stale_writers_and_wrong_proof_boundaries_do_not_replace_work() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ranges.sqlite");
    let limits = RangeLimits::default();
    let mut store = queue(&path, limits);
    let mut stale = RangeStore::open(&path, policy(), limits).unwrap();
    let a = store.advance(blocks(1, 1)).unwrap();
    assert!(stale.advance(blocks(1, 2)).is_err());
    assert_eq!(stale.candidate().height().unwrap(), 0);
    let b = store.advance(blocks(2, 2)).unwrap();
    let wrong = store.batch(b).unwrap().claim.clone();
    assert!(store.retain_range(a, &wrong, vec![1]).is_err());
    assert!(
        store
            .retain_range(a, &store.batch(a).unwrap().claim.clone(), Vec::new())
            .is_err()
    );
    assert!(store.aggregation(&[a, b]).is_err());
    retain(&mut store, b);
    retain(&mut store, a);
    assert!(store.aggregation(&[b, a]).is_err());
    assert!(store.aggregation(&[a, a]).is_err());
    let mut unknown = store.batch(b).unwrap().claim.end.clone();
    unknown.state = Digest([91; 32]);
    assert!(store.retain_prefix(&unknown, vec![1]).is_err());
    drop(store);
    drop(stale);
    let store = RangeStore::open(&path, policy(), limits).unwrap();
    assert_eq!(store.candidate().height().unwrap(), 2);
    assert_eq!(store.batches().len(), 2);
}
#[test]
fn corrupt_leaves_range_boundaries_and_interrupted_writes_fail_closed() {
    for corruption in 0..3 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ranges.sqlite");
        let mut store = queue(&path, RangeLimits::default());
        store.advance(blocks(1, 2)).unwrap();
        store.advance(blocks(3, 4)).unwrap();
        drop(store);
        let db = rusqlite::Connection::open(&path).unwrap();
        match corruption {
            0 => {
                db.execute("DELETE FROM leaves WHERE tree=0", []).unwrap();
            }
            1 => {
                db.execute("DELETE FROM range_batches WHERE id=1", [])
                    .unwrap();
            }
            _ => {
                db.execute("UPDATE range_batches SET witness=x'00' WHERE id=1", [])
                    .unwrap();
            }
        }
        drop(db);
        assert!(RangeStore::open(&path, policy(), RangeLimits::default()).is_err());
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ranges.sqlite");
    let mut store = queue(&path, RangeLimits::default());
    store.advance(blocks(1, 2)).unwrap();
    drop(store);
    let mut db = rusqlite::Connection::open(&path).unwrap();
    {
        let tx = db.transaction().unwrap();
        tx.execute("DELETE FROM range_batches", []).unwrap();
        tx.execute("DELETE FROM leaves", []).unwrap();
    }
    drop(db);
    let store = RangeStore::open(&path, policy(), RangeLimits::default()).unwrap();
    assert_eq!(store.batches().len(), 1);
    assert_eq!(store.candidate().height().unwrap(), 2);
    assert!(
        RangeStore::open(
            &path,
            SourcePolicy {
                confirmations: 4,
                ..policy()
            },
            RangeLimits::default()
        )
        .is_err()
    );
}

#[test]
fn external_durability_failure_rolls_back_state_and_pending_work() {
    use zutti_source::storage::{
        Error,
        durable::{RangeCommit, RangeDurability},
    };
    #[derive(Debug)]
    struct Reject;
    impl RangeDurability for Reject {
        fn commit(&self, _: &RangeCommit) -> zutti_source::storage::Result<()> {
            Err(Error::Persistence)
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.sqlite");
    let mut store = queue(&path, RangeLimits::default());
    let before = store.archive().unwrap();
    store.attach_durability(Box::new(Reject));
    assert!(store.advance(blocks(1, 2)).is_err());
    assert_eq!(store.candidate().height().unwrap(), 0);
    assert_eq!(store.archive().unwrap(), before);
    drop(store);
    let mut store = RangeStore::open(&path, policy(), RangeLimits::default()).unwrap();
    let id = store.advance(blocks(1, 2)).unwrap();
    let claim = store.batch(id).unwrap().claim.clone();
    let before = store.archive().unwrap();
    store.attach_durability(Box::new(Reject));
    assert!(store.retain_range(id, &claim, vec![1]).is_err());
    assert_eq!(store.archive().unwrap(), before);
    drop(store);
    let store = RangeStore::open(&path, policy(), RangeLimits::default()).unwrap();
    assert!(store.batch(id).unwrap().witness.is_some());
    assert!(store.batch(id).unwrap().evidence.is_none());
}
#[test]
fn logical_archive_restores_all_roots_and_pending_witnesses() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.sqlite");
    let mut store = queue(&path, RangeLimits::default());
    store.advance(blocks(1, 2)).unwrap();
    store.advance(blocks(3, 4)).unwrap();
    let archive = store.archive().unwrap();
    let mut leaves = Vec::new();
    store
        .for_each_leaf(&mut |row| {
            leaves.push(row);
            Ok(())
        })
        .unwrap();
    let restored = RangeStore::restore(
        &dir.path().join("restored.sqlite"),
        &archive,
        policy(),
        RangeLimits::default(),
        |put| {
            for row in leaves {
                put(row)?;
            }
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(restored.archive().unwrap(), archive);
    assert_eq!(
        restored
            .candidate()
            .state_snapshot()
            .unwrap()
            .to_bytes()
            .unwrap(),
        store
            .candidate()
            .state_snapshot()
            .unwrap()
            .to_bytes()
            .unwrap()
    );
    let invalid = RangeStore::restore(
        &dir.path().join("invalid.sqlite"),
        &archive,
        policy(),
        RangeLimits::default(),
        |_| Ok(()),
    );
    assert!(invalid.is_err());
}
