#![allow(clippy::unwrap_used)]

use bincode::Options;
use risc0_zkvm::{FakeReceipt, InnerReceipt, Receipt, ReceiptClaim};
use zcash_protocol::consensus::NetworkType;
use zutti_protocol::{encoding::Canonical, statement::ProofKind};
use zutti_prover::{
    Error, ProofEnvelope, ProofVerifier,
    artifact::ProofArtifact,
    source::{
        MAX_SELF_HOSTED_SOURCE_CYCLES, MAX_SOURCE_CYCLES, SourceJob, SourceLimits, policy,
        source_image_id,
    },
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
fn self_hosted_limits_preserve_defaults_and_reject_unbounded_work() {
    assert_eq!(
        SourceLimits::default(),
        SourceLimits::new(MAX_SOURCE_CYCLES, 20).unwrap()
    );
    assert!(SourceLimits::new(100_000_000_000, 22).is_ok());
    for cycles in [0, MAX_SELF_HOSTED_SOURCE_CYCLES + 1, u64::MAX] {
        assert!(SourceLimits::new(cycles, 20).is_err());
    }
    for segment in [0, 19, 23, u32::MAX] {
        assert!(SourceLimits::new(MAX_SOURCE_CYCLES, segment).is_err());
    }
    let job = SourceJob::transition(&witness(), None).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let large = directory.path().join("large");
    job.export_public(&large, 100_000_000_000).unwrap();
    assert!(job.matches(&SourceJob::read_public(&large).unwrap()));
    let rejected = directory.path().join("rejected");
    assert!(
        job.export_public(&rejected, MAX_SELF_HOSTED_SOURCE_CYCLES + 1)
            .is_err()
    );
    assert!(!rejected.exists());
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
    for chain in [11155111_u64, 84532] {
        let mut testnet_result = value.clone();
        testnet_result["chain_id"] = chain.into();
        let path = directory.path().join(format!("result-{chain}.json"));
        zutti_wallet::backup::persist_new(&path, &serde_json::to_vec(&testnet_result).unwrap())
            .unwrap();
        zutti_prover::remote::import_groth16(&path, &expected)
            .unwrap()
            .verify(&expected)
            .unwrap();
        testnet_result["seal"] = "00".repeat(260).into();
        let tampered = directory.path().join(format!("tampered-{chain}.json"));
        zutti_wallet::backup::persist_new(&tampered, &serde_json::to_vec(&testnet_result).unwrap())
            .unwrap();
        assert!(zutti_prover::remote::import_groth16(&tampered, &expected).is_err());
    }
    let seal = zutti_prover::source::groth16_seal(&artifact, &expected).unwrap();
    assert_eq!(hex::encode(seal), value["seal"].as_str().unwrap());
    let proof_path = directory.path().join("source.proof");
    artifact.persist_new(&proof_path).unwrap();
    let binary = env!("CARGO_BIN_EXE_source-prover");
    assert!(
        std::process::Command::new(binary)
            .arg("verify")
            .arg("--proof")
            .arg(&proof_path)
            .output()
            .unwrap()
            .status
            .success()
    );
    let different_job = directory.path().join("different-job");
    SourceJob::transition(&witness(), None)
        .unwrap()
        .export_public(&different_job, MAX_SOURCE_CYCLES)
        .unwrap();
    let rejected_seal = directory.path().join("rejected-seal.bin");
    assert!(
        !std::process::Command::new(binary)
            .arg("verify")
            .arg("--proof")
            .arg(&proof_path)
            .arg("--job")
            .arg(&different_job)
            .arg("--seal")
            .arg(&rejected_seal)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(!rejected_seal.exists());
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
    let genesis =
        GenesisBlock::parse(zebra_test::vectors::MAINNET_BLOCKS[&0], NetworkType::Main).unwrap();
    let mut native =
        SourceCandidate::from_genesis(&genesis, claim.policy, claim.index.context.start()).unwrap();
    native
        .record_transition(
            (1..=4)
                .map(|height| zebra_test::vectors::MAINNET_BLOCKS[&height].to_vec())
                .collect(),
        )
        .unwrap();
    let snapshot = native.state_snapshot().unwrap();
    assert_eq!(snapshot.commitment().unwrap(), claim.state);
    assert_eq!(snapshot.archive(), claim.archive);
    assert_eq!(snapshot.index(), claim.index);
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
