#![allow(clippy::unwrap_used)]
use super::*;
use crate::source::SourcePrograms;
use zcash_protocol::consensus::NetworkType;

struct TestDatabase(MongoDatabase);
impl TestDatabase {
    fn new() -> Self {
        let uri = std::env::var("ZUTTI_TEST_MONGODB_URI").unwrap();
        let nonce = tempfile::tempdir().unwrap();
        let suffix = nonce
            .path()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .replace('.', "");
        Self(MongoDatabase::connect(&uri, &format!("zutti_test_{suffix}"), "source").unwrap())
    }
}
impl Drop for TestDatabase {
    fn drop(&mut self) {
        self.0.database.drop().run().unwrap();
    }
}
fn block(height: u32) -> Vec<u8> {
    zebra_test::vectors::TESTNET_BLOCKS[&height].to_vec()
}
fn policy() -> SourcePolicy {
    SourcePrograms::current().policy(NetworkType::Test, 0, 3)
}
fn initial(path: &Path) -> RangeStore {
    RangeStore::create(path, block(0), policy(), RangeLimits::default()).unwrap()
}
#[test]
#[ignore = "requires a dedicated MongoDB replica set in ZUTTI_TEST_MONGODB_URI"]
fn mongodb_restores_pending_work_and_rejects_stale_writers() {
    let db = TestDatabase::new();
    let dir = tempfile::tempdir().unwrap();
    let original = initial(&dir.path().join("original.sqlite"));
    db.0.initialize(&original).unwrap();
    db.0.initialize(&original).unwrap();
    let a = dir.path().join("a.sqlite");
    let b = dir.path().join("b.sqlite");
    let mut first = db.0.open(&a, policy(), RangeLimits::default()).unwrap();
    assert!(db.0.open(&a, policy(), RangeLimits::default()).is_err());
    let mut stale = db.0.open(&b, policy(), RangeLimits::default()).unwrap();
    first.advance(vec![block(1), block(2)]).unwrap();
    let archive = first.archive().unwrap();
    assert_eq!(db.0.archive().unwrap(), archive);
    assert!(stale.advance(vec![block(1)]).is_err());
    assert_eq!(stale.candidate().height().unwrap(), 0);
    drop(stale);
    let restored = db.0.open(&b, policy(), RangeLimits::default()).unwrap();
    assert_eq!(restored.archive().unwrap(), archive);
    assert_eq!(restored.candidate().height().unwrap(), 2);
    drop(restored);
    std::fs::remove_file(&b).unwrap();
    let restored = db.0.open(&b, policy(), RangeLimits::default()).unwrap();
    assert_eq!(restored.archive().unwrap(), archive);
}
#[test]
#[ignore = "requires a dedicated MongoDB replica set in ZUTTI_TEST_MONGODB_URI"]
fn mongodb_rejection_rolls_back_the_native_frontier_and_queue() {
    let db = TestDatabase::new();
    let dir = tempfile::tempdir().unwrap();
    let original = initial(&dir.path().join("original.sqlite"));
    db.0.initialize(&original).unwrap();
    let mut store =
        db.0.open(
            &dir.path().join("cache.sqlite"),
            policy(),
            RangeLimits::default(),
        )
        .unwrap();
    let before = store.archive().unwrap();
    db.0.database.run_command(doc! {"collMod":"source_metadata", "validator":{"revision":{"$eq":0}},"validationLevel":"strict","validationAction":"error"}).run().unwrap();
    assert!(store.advance(vec![block(1), block(2)]).is_err());
    assert_eq!(store.archive().unwrap(), before);
    assert_eq!(db.0.archive().unwrap(), before);
    assert_eq!(store.candidate().height().unwrap(), 0);
    assert!(store.batches().is_empty());
}
#[test]
#[ignore = "requires a dedicated MongoDB replica set in ZUTTI_TEST_MONGODB_URI"]
fn mongodb_chunks_reject_missing_and_corrupt_content() {
    let db = TestDatabase::new();
    let value = (0..CHUNK * 3 + 17)
        .map(|i| (i % 251) as u8)
        .collect::<Vec<_>>();
    let reference = db.0.put_blob(&value).unwrap();
    assert_eq!(db.0.get_blob(&reference, value.len()).unwrap(), value);
    assert!(db.0.get_blob(&reference, value.len() - 1).is_err());
    let digest = reference.get_str("digest").unwrap();
    db.0.collection("source_blobs")
        .delete_one(doc! {"_id":format!("{digest}:1")})
        .run()
        .unwrap();
    assert!(db.0.get_blob(&reference, value.len()).is_err());
    db.0.put_blob(&value).unwrap();
    db.0.collection("source_blobs")
        .update_one(
            doc! {"_id":format!("{digest}:0")},
            doc! {"$set":{"bytes":binary(&vec![1;CHUNK])}},
        )
        .run()
        .unwrap();
    assert!(db.0.get_blob(&reference, value.len()).is_err());
    let empty = db.0.put_blob(&[]).unwrap();
    assert!(db.0.get_blob(&empty, 0).unwrap().is_empty());
}

#[test]
#[ignore = "requires a dedicated MongoDB replica set in ZUTTI_TEST_MONGODB_URI"]
fn mongodb_restore_rechecks_genuine_receipts_and_authenticated_leaves() {
    use crate::{artifact::ProofArtifact, bootstrap::SourceBootstrap, remote::import_groth16};
    use zutti_protocol::statement::{ProofKind, Statement};
    use zutti_source::proof::SourceReceiptClaim;
    let fixture = include_str!("../../../../tests/fixtures/source-ranges-genesis-proof.json");
    let value: serde_json::Value = serde_json::from_str(fixture).unwrap();
    let payload = hex::decode(value["journal"].as_str().unwrap()).unwrap();
    let claim = SourceReceiptClaim::from_bytes(&payload).unwrap();
    let statement = Statement::new(
        ProofKind::SourceTransition,
        claim.policy,
        claim.program,
        payload,
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let proof_path = dir.path().join("proof.json");
    zutti_wallet::backup::persist_new(&proof_path, fixture.as_bytes()).unwrap();
    let artifact: ProofArtifact = import_groth16(&proof_path, &statement).unwrap();
    let programs = SourcePrograms::current();
    assert_eq!(claim.program, policy().program);
    let original_path = dir.path().join("original.sqlite");
    let mut original = SourceBootstrap::create(
        &original_path,
        block(0),
        policy(),
        programs.clone(),
        RangeLimits::default(),
    )
    .unwrap();
    original.accept_prefix(&artifact).unwrap();
    original.advance(vec![block(1), block(2)]).unwrap();
    let db = TestDatabase::new();
    db.0.initialize(original.store()).unwrap();
    let cache = dir.path().join("restored.sqlite");
    let restored = SourceBootstrap::from_store(
        db.0.open(&cache, policy(), RangeLimits::default()).unwrap(),
        programs.clone(),
    )
    .unwrap();
    assert_eq!(
        original.store().archive().unwrap(),
        restored.store().archive().unwrap()
    );
    assert!(restored.store().evidence().is_some());
    drop(restored);
    std::fs::remove_file(&cache).unwrap();
    db.0.collection("source_leaves")
        .delete_one(doc! {"store":&db.0.store})
        .run()
        .unwrap();
    assert!(db.0.open(&cache, policy(), RangeLimits::default()).is_err());
}
