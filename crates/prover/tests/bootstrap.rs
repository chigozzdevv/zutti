#![allow(clippy::unwrap_used)]
use bincode::Options;
use risc0_zkvm::{FakeReceipt, InnerReceipt, Receipt, ReceiptClaim};
use zcash_protocol::consensus::NetworkType;
use zutti_protocol::{encoding::Canonical, statement::ProofKind};
use zutti_prover::{
    ProofEnvelope,
    artifact::ProofArtifact,
    bootstrap::SourceBootstrap,
    source::{SourceJob, SourcePrograms},
};
use zutti_source::storage::{RangeLimits, RangeStore};
fn fake(job: &SourceJob) -> ProofArtifact {
    let statement = job.statement();
    let receipt = Receipt::new(
        InnerReceipt::Fake(FakeReceipt::new(ReceiptClaim::ok(
            statement.program().0,
            statement.payload().to_vec(),
        ))),
        statement.payload().to_vec(),
    );
    ProofArtifact::new(
        statement.clone(),
        ProofEnvelope::new(
            statement,
            bincode::DefaultOptions::new()
                .with_fixint_encoding()
                .serialize(&receipt)
                .unwrap(),
        )
        .unwrap(),
    )
    .unwrap()
}
fn block(height: u32) -> Vec<u8> {
    zebra_test::vectors::TESTNET_BLOCKS[&height].to_vec()
}
#[test]
fn parallel_jobs_resume_without_accepting_fake_ranges_or_fake_prefixes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bootstrap.sqlite");
    let programs = SourcePrograms::current();
    let policy = programs.policy(NetworkType::Test, 0, 3);
    let limits = RangeLimits::default();
    let mut bootstrap =
        SourceBootstrap::create(&path, block(0), policy, programs.clone(), limits).unwrap();
    let genesis = fake(&bootstrap.genesis_job().unwrap());
    let first = bootstrap.advance(vec![block(1), block(2)]).unwrap();
    let second = bootstrap.advance(vec![block(3), block(4)]).unwrap();
    let statement = bootstrap
        .range_job(second)
        .unwrap()
        .statement()
        .to_bytes()
        .unwrap();
    let range = fake(&bootstrap.range_job(first).unwrap());
    assert_eq!(range.statement().kind(), ProofKind::SourceRange);
    assert!(bootstrap.accept_prefix(&genesis).is_err());
    assert!(bootstrap.accept_prefix(&range).is_err());
    assert!(bootstrap.accept_range(first, &range).is_err());
    assert!(bootstrap.accept_range(second, &range).is_err());
    assert!(bootstrap.aggregate_job(&[first, second]).is_err());
    assert!(bootstrap.fold_job(first).is_err());
    drop(bootstrap);
    let bootstrap = SourceBootstrap::open(&path, policy, programs.clone(), limits).unwrap();
    assert_eq!(
        bootstrap
            .range_job(second)
            .unwrap()
            .statement()
            .to_bytes()
            .unwrap(),
        statement
    );
    assert_eq!(bootstrap.store().candidate().height().unwrap(), 4);
    assert!(bootstrap.store().evidence().is_none());
    drop(bootstrap);
    let mut raw = RangeStore::open(&path, policy, limits).unwrap();
    let claim = raw.batch(first).unwrap().claim.clone();
    raw.retain_range(first, &claim, range.to_bytes().unwrap())
        .unwrap();
    drop(raw);
    assert!(SourceBootstrap::open(&path, policy, programs, limits).is_err());
}
#[test]
fn a_forged_retained_prefix_is_rejected_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bootstrap.sqlite");
    let programs = SourcePrograms::current();
    let policy = programs.policy(NetworkType::Test, 0, 3);
    let limits = RangeLimits::default();
    let bootstrap =
        SourceBootstrap::create(&path, block(0), policy, programs.clone(), limits).unwrap();
    let genesis = fake(&bootstrap.genesis_job().unwrap());
    drop(bootstrap);
    let mut raw = RangeStore::open(&path, policy, limits).unwrap();
    let base = raw.base().clone();
    raw.retain_prefix(&base, genesis.to_bytes().unwrap())
        .unwrap();
    drop(raw);
    assert!(SourceBootstrap::open(&path, policy, programs, limits).is_err());
}
