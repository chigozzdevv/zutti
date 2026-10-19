#![cfg(feature = "storage")]
#![allow(clippy::unwrap_used)]
use zcash_protocol::consensus::NetworkType;
use zutti_protocol::{Digest, encoding::Canonical};
use zutti_source::{
    proof::SourcePolicy,
    storage::{RangeLimits, RangeStore, ReplayStore, SourceStore},
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
fn blocks(first: u32, last: u32) -> Vec<Vec<u8>> {
    (first..=last)
        .map(|h| zebra_test::vectors::TESTNET_BLOCKS[&h].to_vec())
        .collect()
}
#[test]
fn native_samples_resume_without_becoming_verified_source_stores() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("replay.sqlite");
    let mut replay = ReplayStore::create(&path, &blocks(0, 0)[0], policy()).unwrap();
    replay.advance(blocks(1, 2)).unwrap();
    let claim = replay.sample(blocks(3, 4)).unwrap();
    let witness = replay.witness(2).unwrap().unwrap().to_bytes().unwrap();
    assert_eq!(replay.samples().unwrap(), [(2, 4)]);
    drop(replay);
    let replay = ReplayStore::open(&path, policy()).unwrap();
    assert_eq!(replay.candidate().height().unwrap(), 4);
    assert_eq!(
        replay
            .candidate()
            .state_snapshot()
            .unwrap()
            .commitment()
            .unwrap(),
        claim.end.state
    );
    assert_eq!(
        replay.witness(2).unwrap().unwrap().to_bytes().unwrap(),
        witness
    );
    assert!(RangeStore::open(&path, policy(), RangeLimits::default()).is_err());
    assert!(SourceStore::open(&path, policy()).is_err());
}
#[test]
fn invalid_batches_and_stale_writers_preserve_the_committed_frontier() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("replay.sqlite");
    let mut replay = ReplayStore::create(&path, &blocks(0, 0)[0], policy()).unwrap();
    let mut stale = ReplayStore::open(&path, policy()).unwrap();
    assert!(replay.sample(blocks(2, 2)).is_err());
    assert_eq!(replay.candidate().height().unwrap(), 0);
    assert!(replay.samples().unwrap().is_empty());
    replay.advance(blocks(1, 2)).unwrap();
    assert!(stale.advance(blocks(1, 1)).is_err());
    assert_eq!(stale.candidate().height().unwrap(), 0);
    drop(replay);
    assert_eq!(
        ReplayStore::open(&path, policy())
            .unwrap()
            .candidate()
            .height()
            .unwrap(),
        2
    );
    assert!(
        ReplayStore::open(
            &path,
            SourcePolicy {
                index_start: 1,
                ..policy()
            }
        )
        .is_err()
    );
}
#[test]
fn corrupted_leaf_and_sample_data_are_rejected() {
    for sample in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("replay.sqlite");
        let mut replay = ReplayStore::create(&path, &blocks(0, 0)[0], policy()).unwrap();
        replay.sample(blocks(1, 3)).unwrap();
        drop(replay);
        let db = rusqlite::Connection::open(&path).unwrap();
        if sample {
            db.execute("UPDATE replay_samples SET witness=x'00'", [])
                .unwrap();
            drop(db);
            assert!(
                ReplayStore::open(&path, policy())
                    .unwrap()
                    .witness(0)
                    .is_err()
            );
        } else {
            db.execute("DELETE FROM leaves WHERE tree=0", []).unwrap();
            drop(db);
            assert!(ReplayStore::open(&path, policy()).is_err());
        }
    }
}
