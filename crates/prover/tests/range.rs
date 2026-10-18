#![allow(clippy::unwrap_used)]
use bincode::Options;
use risc0_zkvm::{FakeReceipt, InnerReceipt, Receipt, ReceiptClaim};
use zcash_protocol::consensus::NetworkType;
use zutti_protocol::{
    encoding::Canonical,
    statement::{ProofKind, Statement},
};
use zutti_prover::{
    ProofEnvelope, ProofVerifier,
    artifact::ProofArtifact,
    pipeline::SourcePipeline,
    source::{SourceJob, SourcePrograms, source_statement},
};
use zutti_source::{
    bootstrap::GenesisBlock,
    proof::{SourceInput, SourcePolicy, SourceWitness},
    range::SourceRangeWitness,
    state::SourceCandidate,
};

fn fixtures() -> (
    SourcePrograms,
    SourcePolicy,
    SourceWitness,
    Vec<SourceRangeWitness>,
) {
    let programs = SourcePrograms::current();
    let policy = programs.policy(NetworkType::Test, 0, 3);
    let bytes = zebra_test::vectors::TESTNET_BLOCKS[&0].to_vec();
    let genesis = SourceWitness {
        policy,
        input: SourceInput::Genesis(bytes.clone()),
    };
    let block = GenesisBlock::parse(&bytes, policy.network).unwrap();
    let mut source = SourceCandidate::from_genesis(&block, policy.id().unwrap(), 0).unwrap();
    let ranges = [1_u32, 3]
        .map(|first| {
            let transition = source
                .record_transition(
                    (first..first + 2)
                        .map(|h| zebra_test::vectors::TESTNET_BLOCKS[&h].to_vec())
                        .collect(),
                )
                .unwrap();
            SourceRangeWitness::transition(policy, transition).unwrap()
        })
        .to_vec();
    (programs, policy, genesis, ranges)
}
fn fake(statement: &Statement) -> ProofArtifact {
    let receipt = Receipt::new(
        InnerReceipt::Fake(FakeReceipt::new(ReceiptClaim::ok(
            statement.program().0,
            statement.payload().to_vec(),
        ))),
        statement.payload().to_vec(),
    );
    let bytes = bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .serialize(&receipt)
        .unwrap();
    ProofArtifact::new(
        statement.clone(),
        ProofEnvelope::new(statement, bytes).unwrap(),
    )
    .unwrap()
}

#[test]
fn partial_jobs_resume_without_predecessor_proofs_and_cannot_become_full_prefixes() {
    let (programs, policy, _, witnesses) = fixtures();
    let range = &witnesses[1];
    let job = SourceJob::range(range, &[], &programs).unwrap();
    assert_eq!(job.statement().kind(), ProofKind::SourceRange);
    let claim = range.evaluate().unwrap();
    assert_eq!((claim.start.height, claim.end.height), (2, 4));
    let root = tempfile::tempdir().unwrap();
    let out = root.path().join("range");
    job.export_public(&out, 100_000_000).unwrap();
    assert!(job.matches(&SourceJob::read_public(&out).unwrap()));
    let artifact = fake(job.statement());
    assert!(
        ProofVerifier
            .verify(artifact.proof(), artifact.statement())
            .is_err()
    );
    assert!(
        ProofVerifier
            .verify(artifact.proof(), &source_statement(&claim.end).unwrap())
            .is_err()
    );
    let mut pipeline = SourcePipeline::create(
        &root.path().join("source.sqlite"),
        zebra_test::vectors::TESTNET_BLOCKS[&0].to_vec(),
        policy,
        programs,
    )
    .unwrap();
    assert!(pipeline.accept(&artifact).is_err());
    assert!(pipeline.store().evidence().is_none());
    assert!(pipeline.store().pending().is_some());
}

#[test]
fn aggregation_and_folding_require_all_genuine_bound_receipts() {
    let (programs, policy, genesis, witnesses) = fixtures();
    let claims = witnesses
        .iter()
        .map(|w| w.evaluate().unwrap())
        .collect::<Vec<_>>();
    let aggregate = SourceRangeWitness::Aggregate {
        policy,
        ranges: claims.clone(),
    };
    assert!(SourceJob::range(&aggregate, &[], &programs).is_err());
    let jobs = witnesses
        .iter()
        .map(|w| SourceJob::range(w, &[], &programs).unwrap())
        .collect::<Vec<_>>();
    let proofs = jobs.iter().map(|j| fake(j.statement())).collect::<Vec<_>>();
    assert!(SourceJob::range(&aggregate, &[&proofs[0]], &programs).is_err());
    assert!(SourceJob::range(&aggregate, &[&proofs[0], &proofs[1]], &programs).is_err());
    let fold = SourceWitness {
        policy,
        input: SourceInput::Fold {
            previous: Box::new(genesis.evaluate().unwrap()),
            range: Box::new(aggregate.evaluate().unwrap()),
        },
    };
    assert!(SourceJob::transition_with(&fold, None, &programs).is_err());
    let prefix = fake(&source_statement(&genesis.evaluate().unwrap()).unwrap());
    assert!(SourceJob::fold(&fold, &prefix, &proofs[0], &programs).is_err());
    let extra = SourceRangeWitness::Transition(Box::new(genesis));
    assert!(SourceJob::range(&extra, &[], &programs).is_err());
}

#[test]
fn genuine_canonical_prefix_remains_distinct_from_a_conditional_range() {
    let value: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/source-testnet-64-proof.json"
    ))
    .unwrap();
    let payload = hex::decode(value["journal"].as_str().unwrap()).unwrap();
    let claim = zutti_source::proof::SourceReceiptClaim::from_bytes(&payload).unwrap();
    assert_eq!(claim.height, 64);
    assert_eq!(
        claim.genesis,
        zutti_source::bootstrap::genesis_hash(NetworkType::Test).unwrap()
    );
    let statement = Statement::new(
        ProofKind::SourceTransition,
        claim.policy,
        claim.program,
        payload.clone(),
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("result.json");
    zutti_wallet::backup::persist_new(&path, &serde_json::to_vec(&value).unwrap()).unwrap();
    let artifact = zutti_prover::remote::import_groth16(&path, &statement).unwrap();
    artifact.verify(&statement).unwrap();
    let wrong =
        Statement::new(ProofKind::SourceRange, claim.policy, claim.program, payload).unwrap();
    let proof = ProofEnvelope::new(&wrong, artifact.proof().proof().to_vec()).unwrap();
    assert!(ProofVerifier.verify(&proof, &wrong).is_err());
}
