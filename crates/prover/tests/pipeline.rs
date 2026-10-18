#![allow(clippy::unwrap_used)]
use bincode::Options;
use risc0_zkvm::{FakeReceipt, InnerReceipt, Receipt, ReceiptClaim};
use zcash_protocol::consensus::NetworkType;
use zutti_protocol::encoding::Canonical;
use zutti_prover::{
    ProofEnvelope, artifact::ProofArtifact, pipeline::SourcePipeline, source::SourcePrograms,
};

#[test]
fn pending_genesis_survives_restart_and_rejects_fake_or_unverified_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("source.sqlite");
    let programs = SourcePrograms::current();
    let policy = programs.policy(NetworkType::Test, 0, 3);
    let mut pipeline = SourcePipeline::create(
        &database,
        zebra_test::vectors::TESTNET_BLOCKS[&0].to_vec(),
        policy,
        programs.clone(),
    )
    .unwrap();
    let job = pipeline.job().unwrap();
    let expected = job.statement().to_bytes().unwrap();
    let receipt = Receipt::new(
        InnerReceipt::Fake(FakeReceipt::new(ReceiptClaim::ok(
            policy.program.0,
            job.statement().payload().to_vec(),
        ))),
        job.statement().payload().to_vec(),
    );
    let proof = ProofEnvelope::new(
        job.statement(),
        bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .serialize(&receipt)
            .unwrap(),
    )
    .unwrap();
    let artifact = ProofArtifact::new(job.statement().clone(), proof).unwrap();
    assert!(pipeline.accept(&artifact).is_err());
    assert!(
        pipeline
            .advance(vec![zebra_test::vectors::TESTNET_BLOCKS[&1].to_vec()])
            .is_err()
    );
    drop(pipeline);
    let pipeline = SourcePipeline::open(&database, policy, programs.clone()).unwrap();
    assert_eq!(
        pipeline.job().unwrap().statement().to_bytes().unwrap(),
        expected
    );
    assert!(pipeline.store().evidence().is_none());
    drop(pipeline);
    let mut store = zutti_source::storage::SourceStore::open(&database, policy).unwrap();
    let state = store
        .candidate()
        .state_snapshot()
        .unwrap()
        .commitment()
        .unwrap();
    store
        .retain_evidence(state, artifact.to_bytes().unwrap())
        .unwrap();
    drop(store);
    assert!(SourcePipeline::open(&database, policy, programs).is_err());
}

#[test]
fn retained_testnet_genesis_receipt_matches_the_exact_native_policy_and_state() {
    for (fixture, registry) in [
        (
            include_str!("../../../tests/fixtures/source-testnet-genesis-proof.json"),
            "26797bf40bd99349e33061bd5826bdf8dc05a5200b085f1e859c33281cc40710",
        ),
        (
            include_str!("../../../tests/fixtures/source-nu7-genesis-proof.json"),
            "7742c955d72428f0c643a7d92cd4aa81567779a46c48d2e6e24c2c7c2a73667b",
        ),
        (
            include_str!("../../../tests/fixtures/source-canonical-genesis-proof.json"),
            "f5b14f1fa25a45a6ae86316006e802d7adcf1df087ff8f4779da01922c1dc37f",
        ),
    ] {
        check_retained_genesis(fixture, registry);
    }
}

fn check_retained_genesis(fixture: &str, registry: &str) {
    use zutti_protocol::{
        Digest,
        statement::{ProofKind, Statement},
    };
    use zutti_source::proof::{SourceInput, SourcePolicy, SourceReceiptClaim, SourceWitness};
    let value: serde_json::Value = serde_json::from_str(fixture).unwrap();
    let payload = hex::decode(value["journal"].as_str().unwrap()).unwrap();
    let claim = SourceReceiptClaim::from_bytes(&payload).unwrap();
    let policy = SourcePolicy {
        program: claim.program,
        registry_program: Digest::from_hex(registry).unwrap(),
        network: NetworkType::Test,
        index_start: 0,
        confirmations: 100,
    };
    let expected = SourceWitness {
        policy,
        input: SourceInput::Genesis(zebra_test::vectors::TESTNET_BLOCKS[&0].to_vec()),
    }
    .evaluate()
    .unwrap();
    assert_eq!(claim, expected);
    let statement = Statement::new(
        ProofKind::SourceTransition,
        policy.id().unwrap(),
        claim.program,
        payload,
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let result = directory.path().join("result.json");
    zutti_wallet::backup::persist_new(&result, &serde_json::to_vec(&value).unwrap()).unwrap();
    let artifact = zutti_prover::remote::import_groth16(&result, &statement).unwrap();
    artifact.verify(&statement).unwrap();
    let mut altered = value.clone();
    let mut seal = hex::decode(altered["seal"].as_str().unwrap()).unwrap();
    seal[50] ^= 1;
    altered["seal"] = hex::encode(seal).into();
    let path = directory.path().join("altered.json");
    zutti_wallet::backup::persist_new(&path, &serde_json::to_vec(&altered).unwrap()).unwrap();
    assert!(zutti_prover::remote::import_groth16(&path, &statement).is_err());
}
