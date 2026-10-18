#![allow(clippy::unwrap_used)]
use zcash_protocol::consensus::NetworkType;
use zutti_protocol::{
    encoding::Canonical,
    statement::{ProofKind, Statement},
};
use zutti_prover::{
    artifact::ProofArtifact, bootstrap::SourceBootstrap, remote::import_groth16,
    source::SourcePrograms,
};
use zutti_source::{proof::SourceReceiptClaim, range::SourceRangeClaim, storage::RangeLimits};
fn import(fixture: &str, kind: ProofKind) -> ProofArtifact {
    let value: serde_json::Value = serde_json::from_str(fixture).unwrap();
    let payload = hex::decode(value["journal"].as_str().unwrap()).unwrap();
    let (program, domain) = if kind == ProofKind::SourceRange {
        let claim = SourceRangeClaim::from_bytes(&payload).unwrap();
        (claim.start.program, claim.start.policy)
    } else {
        let claim = SourceReceiptClaim::from_bytes(&payload).unwrap();
        (claim.program, claim.policy)
    };
    let statement = Statement::new(kind, domain, program, payload).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("proof.json");
    zutti_wallet::backup::persist_new(&path, fixture.as_bytes()).unwrap();
    import_groth16(&path, &statement).unwrap()
}
fn blocks(first: u32, last: u32) -> Vec<Vec<u8>> {
    (first..=last)
        .map(|h| zebra_test::vectors::TESTNET_BLOCKS[&h].to_vec())
        .collect()
}
#[test]
#[ignore = "requires the retained source range program ELFs in ZUTTI_RANGE_PROGRAMS"]
fn genuine_receipts_complete_and_restart_the_durable_bootstrap_queue() {
    let programs = SourcePrograms::read(std::path::Path::new(
        &std::env::var_os("ZUTTI_RANGE_PROGRAMS").unwrap(),
    ))
    .unwrap();
    let policy = programs.policy(NetworkType::Test, 0, 3);
    let limits = RangeLimits::default();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bootstrap.sqlite");
    let genesis = import(
        include_str!("../../../tests/fixtures/source-ranges-genesis-proof.json"),
        ProofKind::SourceTransition,
    );
    let a = import(
        include_str!("../../../tests/fixtures/source-ranges-0-2-proof.json"),
        ProofKind::SourceRange,
    );
    let b = import(
        include_str!("../../../tests/fixtures/source-ranges-2-4-proof.json"),
        ProofKind::SourceRange,
    );
    let aggregate = import(
        include_str!("../../../tests/fixtures/source-ranges-0-4-proof.json"),
        ProofKind::SourceRange,
    );
    let prefix = import(
        include_str!("../../../tests/fixtures/source-ranges-prefix-4-proof.json"),
        ProofKind::SourceTransition,
    );
    let mut bootstrap = SourceBootstrap::create(
        &path,
        zebra_test::vectors::TESTNET_BLOCKS[&0].to_vec(),
        policy,
        programs.clone(),
        limits,
    )
    .unwrap();
    assert_eq!(
        bootstrap.genesis_job().unwrap().statement(),
        genesis.statement()
    );
    bootstrap.accept_prefix(&genesis).unwrap();
    bootstrap.accept_prefix(&genesis).unwrap();
    let first = bootstrap.advance(blocks(1, 2)).unwrap();
    let second = bootstrap.advance(blocks(3, 4)).unwrap();
    assert_eq!(
        bootstrap.range_job(first).unwrap().statement(),
        a.statement()
    );
    assert_eq!(
        bootstrap.range_job(second).unwrap().statement(),
        b.statement()
    );
    assert!(bootstrap.accept_prefix(&aggregate).is_err());
    bootstrap.accept_range(second, &b).unwrap();
    bootstrap.accept_range(second, &b).unwrap();
    drop(bootstrap);
    let mut bootstrap = SourceBootstrap::open(&path, policy, programs.clone(), limits).unwrap();
    assert_eq!(bootstrap.store().base().height, 0);
    assert!(bootstrap.store().batch(first).unwrap().witness.is_some());
    bootstrap.accept_range(first, &a).unwrap();
    bootstrap.accept_range(first, &a).unwrap();
    assert_eq!(
        bootstrap
            .aggregate_job(&[first, second])
            .unwrap()
            .statement(),
        aggregate.statement()
    );
    bootstrap
        .accept_aggregate(&[first, second], &aggregate)
        .unwrap();
    bootstrap
        .accept_aggregate(&[first, second], &aggregate)
        .unwrap();
    assert_eq!(bootstrap.store().batches().len(), 1);
    assert_eq!(
        bootstrap.fold_job(first).unwrap().statement(),
        prefix.statement()
    );
    bootstrap.accept_prefix(&prefix).unwrap();
    bootstrap.accept_prefix(&prefix).unwrap();
    assert_eq!(bootstrap.store().base().height, 4);
    assert!(bootstrap.store().batches().is_empty());
    drop(bootstrap);
    let mut bootstrap = SourceBootstrap::open(&path, policy, programs.clone(), limits).unwrap();
    assert_eq!(
        bootstrap
            .store()
            .candidate()
            .state_snapshot()
            .unwrap()
            .commitment()
            .unwrap(),
        SourceReceiptClaim::from_bytes(prefix.statement().payload())
            .unwrap()
            .state
    );
    assert!(bootstrap.advance(blocks(5, 5)).unwrap() > second);
    drop(bootstrap);
    let bootstrap = SourceBootstrap::open(&path, policy, programs, limits).unwrap();
    assert_eq!(bootstrap.store().base().height, 4);
    assert_eq!(bootstrap.store().candidate().height().unwrap(), 5);
}
