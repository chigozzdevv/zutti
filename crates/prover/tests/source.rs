#![allow(clippy::unwrap_used)]

use bincode::Options;
use risc0_zkvm::{FakeReceipt, InnerReceipt, Receipt, ReceiptClaim};
use zcash_protocol::consensus::NetworkType;
use zutti_protocol::{encoding::Canonical, statement::ProofKind};
use zutti_prover::{
    Error, ProofEnvelope, ProofVerifier,
    artifact::ProofArtifact,
    source::{SourceJob, policy, source_image_id},
};
use zutti_source::{
    bootstrap::GenesisBlock,
    proof::{SourceInput, SourceWitness},
    state::SourceCandidate,
};

fn genesis() -> Vec<u8> {
    hex::decode(include_str!("../../../tests/fixtures/block-0.txt").trim()).unwrap()
}
fn witness() -> SourceWitness {
    SourceWitness {
        policy: policy(NetworkType::Main, 0, 3),
        input: SourceInput::Genesis(genesis()),
    }
}

#[test]
fn exported_jobs_pin_the_source_program_and_reject_modified_statements() {
    let job = SourceJob::transition(&witness(), None).unwrap();
    assert_eq!(job.statement().kind(), ProofKind::SourceTransition);
    assert_eq!(job.statement().program(), source_image_id());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("job");
    job.export_public(&path, 268_435_456).unwrap();
    assert!(job.export_public(&path, 268_435_456).is_err());
    let loaded = SourceJob::read_public(&path).unwrap();
    assert_eq!(loaded.statement(), job.statement());
    let mut bytes = job.statement().to_bytes().unwrap();
    bytes[43] ^= 1;
    std::fs::write(path.join("statement.bin"), bytes).unwrap();
    assert!(SourceJob::read_public(&path).is_err());
}

#[test]
fn recursive_source_jobs_reject_missing_or_fake_predecessor_proofs() {
    let initial = witness();
    let job = SourceJob::transition(&initial, None).unwrap();
    let receipt = Receipt::new(
        InnerReceipt::Fake(FakeReceipt::new(ReceiptClaim::ok(
            source_image_id().0,
            job.statement().payload().to_vec(),
        ))),
        job.statement().payload().to_vec(),
    );
    let bytes = bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .serialize(&receipt)
        .unwrap();
    let proof = ProofEnvelope::new(job.statement(), bytes).unwrap();
    assert_eq!(
        ProofVerifier.verify(&proof, job.statement()).unwrap_err(),
        Error::DevelopmentReceipt
    );
    let artifact = ProofArtifact::new(job.statement().clone(), proof).unwrap();
    assert!(SourceJob::transition(&initial, Some(&artifact)).is_err());
    let genesis = GenesisBlock::parse(&genesis(), NetworkType::Main).unwrap();
    let mut state =
        SourceCandidate::from_genesis(&genesis, initial.policy.id().unwrap(), 0).unwrap();
    let block = hex::decode(include_str!("../../../tests/fixtures/block-1.txt").trim()).unwrap();
    let transition = state.record_transition(vec![block]).unwrap();
    let next = SourceWitness {
        policy: initial.policy,
        input: SourceInput::Advance {
            previous: Box::new(initial.evaluate().unwrap()),
            transition: Box::new(transition),
        },
    };
    assert!(SourceJob::transition(&next, None).is_err());
    assert_eq!(
        SourceJob::transition(&next, Some(&artifact)).unwrap_err(),
        Error::DevelopmentReceipt
    );
}

#[test]
#[ignore = "executes the full source guest using r0vm"]
fn source_genesis_guest_matches_native_execution() {
    let executable = std::env::var_os("RISC0_SERVER_PATH").unwrap_or_else(|| "r0vm".into());
    let result = zutti_prover::local::LocalProver::new(executable)
        .execute_source(&SourceJob::transition(&witness(), None).unwrap())
        .unwrap();
    assert!(result.journal_matches);
    assert!(!result.proof_generated);
    assert!(result.cycles > 0);
}

#[test]
fn retained_genesis_groth16_receipt_verifies_and_rejects_tampering() {
    let value: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/source-genesis-proof.json"
    ))
    .unwrap();
    let bytes = hex::decode(value["journal"].as_str().unwrap()).unwrap();
    let claim = zutti_source::proof::SourceReceiptClaim::from_bytes(&bytes).unwrap();
    let expected = zutti_protocol::statement::Statement::new(
        ProofKind::SourceTransition,
        claim.policy,
        claim.program,
        bytes,
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("result.json");
    zutti_wallet::backup::persist_new(&path, &serde_json::to_vec(&value).unwrap()).unwrap();
    let artifact = zutti_prover::remote::import_groth16(&path, &expected).unwrap();
    artifact.verify(&expected).unwrap();
    let seal = zutti_prover::source::groth16_seal(&artifact, &expected).unwrap();
    assert_eq!(hex::encode(seal), value["seal"].as_str().unwrap());
    let mut value = value;
    let mut seal = hex::decode(value["seal"].as_str().unwrap()).unwrap();
    seal[259] ^= 1;
    value["seal"] = hex::encode(seal).into();
    let changed = directory.path().join("changed.json");
    zutti_wallet::backup::persist_new(&changed, &serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(zutti_prover::remote::import_groth16(&changed, &expected).is_err());
}

#[test]
fn genuine_recursive_prefix_receipt_proves_the_first_four_blocks() {
    let value: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/source-prefix-proof.json"
    ))
    .unwrap();
    let payload = hex::decode(value["journal"].as_str().unwrap()).unwrap();
    let claim = zutti_source::proof::SourceReceiptClaim::from_bytes(&payload).unwrap();
    assert_eq!(claim.height, 4);
    assert_eq!(claim.index.height, 4);
    let statement = zutti_protocol::statement::Statement::new(
        ProofKind::SourceTransition,
        claim.policy,
        claim.program,
        payload,
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("result.json");
    zutti_wallet::backup::persist_new(&path, &serde_json::to_vec(&value).unwrap()).unwrap();
    zutti_prover::remote::import_groth16(&path, &statement)
        .unwrap()
        .verify(&statement)
        .unwrap();
}
