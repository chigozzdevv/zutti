#![allow(clippy::unwrap_used)]
use zcash_protocol::consensus::NetworkType;
use zutti_protocol::{
    Digest,
    encoding::Canonical,
    pool::SourceUpdateClaim,
    statement::{ProofKind, Statement},
};
use zutti_prover::{admission::SourceAdmission, artifact::ProofArtifact};
use zutti_source::{
    bootstrap::GenesisBlock, checkpoint::SourceCheckpoint, proof::SourcePolicy,
    state::SourceCandidate, transition::StateSnapshot,
};

fn evidence() -> (
    SourceAdmission,
    ProofArtifact,
    StateSnapshot,
    SourceCheckpoint,
) {
    let value: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/source-update-proof.json"
    ))
    .unwrap();
    let payload = hex::decode(value["journal"].as_str().unwrap()).unwrap();
    let claim = SourceUpdateClaim::from_bytes(&payload).unwrap();
    let registry_program = Digest::from_hex(value["image_id"].as_str().unwrap()).unwrap();
    let statement =
        Statement::new(ProofKind::Source, claim.policy, registry_program, payload).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("result.json");
    zutti_wallet::backup::persist_new(&path, &serde_json::to_vec(&value).unwrap()).unwrap();
    let proof = zutti_prover::remote::import_groth16(&path, &statement).unwrap();
    let profile = SourceAdmission {
        policy: SourcePolicy {
            network: NetworkType::Main,
            program: Digest::from_hex(
                "c5a1d868e176647966ce6e728860c9225c99e05a0d8880a474e1aeeb652ec669",
            )
            .unwrap(),
            registry_program,
            index_start: 0,
            confirmations: 3,
        },
        minimum_height: 4,
        minimum_work: claim.work_be,
        maximum_age: 600,
        maximum_future: 30,
    };
    let genesis =
        GenesisBlock::parse(zebra_test::vectors::MAINNET_BLOCKS[&0], NetworkType::Main).unwrap();
    let mut source = SourceCandidate::from_genesis(&genesis, claim.policy, 0).unwrap();
    source
        .record_transition(
            (1..=4)
                .map(|height| zebra_test::vectors::MAINNET_BLOCKS[&height].to_vec())
                .collect(),
        )
        .unwrap();
    (
        profile,
        proof,
        source.state_snapshot().unwrap(),
        source.checkpoint(3).unwrap(),
    )
}

#[test]
fn admits_genuine_history_only_under_the_expected_profile_and_time_bounds() {
    let (profile, proof, state, checkpoint) = evidence();
    let now = u64::from(state.timestamp().unwrap()) + 1;
    let verified = profile
        .verify(&proof, &state, checkpoint.clone(), now)
        .unwrap();
    assert_eq!(verified.state(), state.commitment().unwrap());
    assert_eq!(verified.height(), 4);
    assert_eq!(
        verified.checkpoint(now).unwrap().id().unwrap(),
        checkpoint.id().unwrap()
    );
    assert!(verified.checkpoint(now + 600).is_err());
    assert!(
        profile
            .verify(&proof, &state, checkpoint.clone(), 1_792_281_600)
            .is_err()
    );
    assert!(
        profile
            .verify(&proof, &state, checkpoint.clone(), now - 32)
            .is_err()
    );
    let encoded = profile.to_bytes().unwrap();
    SourceAdmission::from_bytes(&encoded)
        .unwrap()
        .verify(&proof, &state, checkpoint, now)
        .unwrap();
    let mut trailing = encoded;
    trailing.push(0);
    assert!(SourceAdmission::from_bytes(&trailing).is_err());
}

#[test]
fn rejects_changed_source_policy_work_floor_state_and_stable_checkpoint() {
    let (profile, proof, state, checkpoint) = evidence();
    let now = u64::from(state.timestamp().unwrap()) + 1;
    for change in 0..7 {
        let mut altered = profile.clone();
        match change {
            0 => altered.minimum_work[0] = 1,
            1 => altered.minimum_height = 5,
            2 => altered.policy.confirmations = 2,
            3 => altered.policy.index_start = 1,
            4 => altered.policy.registry_program.0[0] ^= 1,
            5 => altered.policy.program.0[0] ^= 1,
            _ => altered.policy.network = NetworkType::Test,
        }
        assert!(
            altered
                .verify(&proof, &state, checkpoint.clone(), now)
                .is_err()
        );
    }
    let mut altered = checkpoint.clone();
    altered.stable.hash.0[0] ^= 1;
    assert!(profile.verify(&proof, &state, altered, now).is_err());
    let mut encoded = state.to_bytes().unwrap();
    let index_root = state.index().root.0;
    let offset = encoded
        .windows(32)
        .position(|bytes| bytes == index_root)
        .unwrap();
    encoded[offset] ^= 1;
    let altered = StateSnapshot::from_bytes(&encoded).unwrap();
    assert!(profile.verify(&proof, &altered, checkpoint, now).is_err());
}

#[test]
fn valid_metadata_cannot_admit_a_development_receipt() {
    use bincode::Options;
    use risc0_zkvm::{FakeReceipt, InnerReceipt, Receipt, ReceiptClaim};
    use zutti_prover::{Error, ProofEnvelope};
    let (profile, proof, state, checkpoint) = evidence();
    let statement = proof.statement().clone();
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
    let envelope = ProofEnvelope::new(&statement, bytes).unwrap();
    let fake = ProofArtifact::new(statement, envelope).unwrap();
    assert!(matches!(
        profile.verify(
            &fake,
            &state,
            checkpoint,
            u64::from(state.timestamp().unwrap()) + 1
        ),
        Err(Error::DevelopmentReceipt)
    ));
}

#[test]
fn admission_cli_uses_the_current_clock_and_refuses_historical_evidence() {
    let (profile, proof, state, checkpoint) = evidence();
    let directory = tempfile::tempdir().unwrap();
    for (name, bytes) in [
        ("profile.bin", profile.to_bytes().unwrap()),
        ("state.bin", state.to_bytes().unwrap()),
        ("checkpoint.bin", checkpoint.to_bytes().unwrap()),
    ] {
        zutti_wallet::backup::persist_new(&directory.path().join(name), &bytes).unwrap();
    }
    proof
        .persist_new(&directory.path().join("source.proof"))
        .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_source-prover"))
        .arg("admit")
        .arg("--profile")
        .arg(directory.path().join("profile.bin"))
        .arg("--state")
        .arg(directory.path().join("state.bin"))
        .arg("--checkpoint")
        .arg(directory.path().join("checkpoint.bin"))
        .arg("--proof")
        .arg(directory.path().join("source.proof"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("source evidence is stale")
    );
}
