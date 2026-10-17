#![allow(clippy::unwrap_used)]
#[path = "../../../tests/support/payment.rs"]
mod payment;
#[path = "../../../tests/support/mod.rs"]
mod support;

use bincode::Options;
use risc0_zkvm::{
    Executor, ExecutorEnv, ExternalProver, FakeReceipt, InnerReceipt, Receipt, ReceiptClaim,
};
use zutti_protocol::{
    Digest,
    encoding::Canonical,
    escrow::ReservationOpening,
    statement::{ProofKind, Statement},
};
use zutti_prover::{
    Error, ProofEnvelope, ProofVerifier,
    artifact::ProofArtifact,
    preparation::{preparation_image_id, preparation_statement},
};
use zutti_wallet::{
    backup,
    preparation::{PreparationClaim, PreparationWitness},
    prepare::inspect_ironwood,
};

fn witness(wrong_memo: bool) -> PreparationWitness {
    let fixture = payment::fixture(wrong_memo);
    PreparationWitness::new(
        fixture.terms,
        fixture.randomness,
        support::secret(32),
        fixture.pczt.serialize().unwrap(),
    )
    .unwrap()
}

fn codec() -> impl Options {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .reject_trailing_bytes()
}

#[test]
fn preparation_binds_the_actual_effects_without_publishing_the_payment() {
    let fixture = payment::fixture(false);
    let bytes = fixture.pczt.serialize().unwrap();
    let inspected = inspect_ironwood(&bytes, &fixture.terms, &fixture.randomness).unwrap();
    let domain = fixture.terms.domain().id().unwrap();
    let order = fixture.terms.commitment(&fixture.randomness).unwrap();
    let reservation =
        ReservationOpening::bind(domain, order, inspected.txid(), &support::secret(32)).unwrap();
    let witness = PreparationWitness::new(
        fixture.terms.clone(),
        fixture.randomness,
        support::secret(32),
        bytes,
    )
    .unwrap();
    let claim = witness.evaluate().unwrap();
    assert_eq!(claim.domain(), domain);
    assert_eq!(claim.order(), order);
    assert_eq!(claim.reservation(), reservation);
    let encoded = claim.to_bytes().unwrap();
    assert_eq!(encoded.len(), 106);
    assert!(!encoded.windows(32).any(|bytes| bytes == inspected.txid().0));
    assert!(
        !encoded
            .windows(43)
            .any(|bytes| bytes == fixture.terms.receiver())
    );
    assert_eq!(PreparationClaim::from_bytes(&encoded).unwrap(), claim);
    assert_eq!(
        PreparationWitness::from_bytes(&witness.to_bytes().unwrap())
            .unwrap()
            .evaluate()
            .unwrap(),
        claim
    );
    let statement = preparation_statement(&claim).unwrap();
    assert_eq!(statement.kind(), ProofKind::Preparation);
    assert_eq!(statement.program(), preparation_image_id());
    assert_eq!(statement.domain(), domain);
    assert_eq!(format!("{witness:?}"), "PreparationWitness([REDACTED])");
}

#[test]
fn malformed_witnesses_and_incorrect_encryption_cannot_be_proved() {
    assert!(witness(true).evaluate().is_err());
    let encoded = witness(false).to_bytes().unwrap();
    for end in [0, 8, 64, encoded.len() - 1] {
        assert!(PreparationWitness::from_bytes(&encoded[..end]).is_err());
    }
    let mut trailing = encoded;
    trailing.push(0);
    assert!(PreparationWitness::from_bytes(&trailing).is_err());
    assert!(
        PreparationWitness::new(
            support::terms(),
            support::secret(21),
            support::secret(32),
            vec![]
        )
        .is_err()
    );
}

#[test]
fn altered_note_ciphertext_and_authentication_tag_are_rejected() {
    let fixture = payment::fixture(false);
    let mut ciphertext = None;
    pczt::roles::verifier::Verifier::new(fixture.pczt.clone())
        .with_ironwood::<(), _>(|bundle| {
            for action in bundle.actions() {
                if action
                    .output()
                    .recipient()
                    .map(|recipient| recipient.to_raw_address_bytes())
                    .as_ref()
                    == Some(fixture.terms.receiver())
                {
                    ciphertext = Some(action.output().encrypted_note().enc_ciphertext);
                }
            }
            Ok(())
        })
        .unwrap()
        .finish();
    let ciphertext = ciphertext.unwrap();
    let original = fixture.pczt.serialize().unwrap();
    let offset = original
        .windows(ciphertext.len())
        .position(|part| part == ciphertext)
        .unwrap();
    for byte in [0, 32, ciphertext.len() - 1] {
        let mut changed = original.clone();
        changed[offset + byte] ^= 1;
        let witness = PreparationWitness::new(
            fixture.terms.clone(),
            support::secret(21),
            support::secret(32),
            changed,
        )
        .unwrap();
        assert!(witness.evaluate().is_err());
    }
}

#[test]
fn preparation_verification_rejects_development_receipts_and_relabelling() {
    let statement = preparation_statement(&witness(false).evaluate().unwrap()).unwrap();
    let receipt = Receipt::new(
        InnerReceipt::Fake(FakeReceipt::new(ReceiptClaim::ok(
            zutti_methods::PREPARATION_ID,
            statement.payload().to_vec(),
        ))),
        statement.payload().to_vec(),
    );
    let bytes = codec().serialize(&receipt).unwrap();
    let envelope = ProofEnvelope::new(&statement, bytes.clone()).unwrap();
    assert_eq!(
        ProofVerifier.verify(&envelope, &statement).unwrap_err(),
        Error::DevelopmentReceipt
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("fake.proof");
    ProofArtifact::new(statement.clone(), envelope)
        .unwrap()
        .persist_new(&path)
        .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_private-prover"))
        .args(["verify-preparation", "--proof"])
        .arg(&path)
        .env("RISC0_DEV_MODE", "1")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let wrong_image = Statement::new(
        ProofKind::Preparation,
        statement.domain(),
        Digest([99; 32]),
        statement.payload().to_vec(),
    )
    .unwrap();
    assert_eq!(
        ProofVerifier
            .verify(
                &ProofEnvelope::new(&wrong_image, bytes.clone()).unwrap(),
                &wrong_image
            )
            .unwrap_err(),
        Error::BackendUnavailable
    );
    let wrong_domain = Statement::new(
        ProofKind::Preparation,
        Digest([99; 32]),
        preparation_image_id(),
        statement.payload().to_vec(),
    )
    .unwrap();
    assert_eq!(
        ProofVerifier
            .verify(
                &ProofEnvelope::new(&wrong_domain, bytes).unwrap(),
                &wrong_domain
            )
            .unwrap_err(),
        Error::Binding
    );
}

#[test]
#[ignore = "executes the RISC Zero preparation guest"]
fn preparation_guest_matches_native_checks_and_rejects_wrong_memo() {
    for wrong_memo in [false, true] {
        let witness = witness(wrong_memo);
        let bytes = zeroize::Zeroizing::new(witness.to_bytes().unwrap());
        let length = u32::try_from(bytes.len()).unwrap().to_le_bytes();
        let env = ExecutorEnv::builder()
            .write_slice(&length)
            .write_slice(bytes.as_slice())
            .session_limit(Some(268_435_456))
            .segment_limit_po2(20)
            .build()
            .unwrap();
        let executable = std::env::var_os("RISC0_SERVER_PATH").unwrap_or_else(|| "r0vm".into());
        let started = std::time::Instant::now();
        let result = ExternalProver::new("zutti-test", executable)
            .execute(env, zutti_methods::PREPARATION_ELF);
        if wrong_memo {
            assert!(result.is_err());
        } else {
            let session = result.unwrap();
            assert_eq!(
                session.journal.bytes,
                witness.evaluate().unwrap().to_bytes().unwrap()
            );
            eprintln!(
                "preparation execution: {} cycles, {} segments, {:.3} s",
                session.cycles(),
                session.segments.len(),
                started.elapsed().as_secs_f64()
            );
        }
    }
}

#[test]
#[ignore = "generates a cryptographic preparation receipt"]
fn real_preparation_proof_verifies_and_tampering_fails() {
    let tools = tempfile::tempdir().unwrap();
    let cli = tools.path().join("private-prover");
    std::fs::copy(env!("CARGO_BIN_EXE_private-prover"), &cli).unwrap();
    let witness = witness(false);
    let claim = witness.evaluate().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let key_file = directory.path().join("recovery.key");
    let archive = directory.path().join("preparation.backup");
    let path = directory.path().join("preparation.proof");
    let key = support::secret(61);
    backup::persist_new(&key_file, key.expose()).unwrap();
    let plaintext = zeroize::Zeroizing::new(witness.to_bytes().unwrap());
    let encrypted = backup::seal(claim.domain(), &plaintext, &key).unwrap();
    backup::persist_new(&archive, &encrypted).unwrap();
    let status = std::process::Command::new(&cli)
        .arg("status")
        .output()
        .unwrap();
    assert!(status.status.success());
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(
        status["preparation_image_id"],
        preparation_image_id().to_string()
    );
    let executable = std::env::var_os("RISC0_SERVER_PATH").unwrap_or_else(|| "r0vm".into());
    let started = std::time::Instant::now();
    let output = std::process::Command::new(&cli)
        .args(["prove-preparation", "--backup"])
        .arg(&archive)
        .arg("--key-file")
        .arg(&key_file)
        .arg("--domain")
        .arg(claim.domain().to_string())
        .arg("--prover")
        .arg(&executable)
        .arg("--out")
        .arg(&path)
        .env("RUST_LOG", "info")
        .env("RISC0_DEV_MODE", "0")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let status: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(status["kind"], "preparation");
    assert_eq!(status["settlement_authority"], false);
    let elapsed = started.elapsed().as_secs_f64();
    let artifact = ProofArtifact::read(&path).unwrap();
    artifact
        .verify(&preparation_statement(&claim).unwrap())
        .unwrap();
    let output = std::process::Command::new(&cli)
        .args(["verify-preparation", "--proof"])
        .arg(&path)
        .env("RISC0_DEV_MODE", "1")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["proof_verified"]
            .as_bool()
            .unwrap()
    );
    let output = std::process::Command::new(&cli)
        .args(["verify-query", "--proof"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let mut receipt: Receipt = codec().deserialize(artifact.proof().proof()).unwrap();
    receipt.journal.bytes[42] ^= 1;
    let changed =
        ProofEnvelope::new(artifact.statement(), codec().serialize(&receipt).unwrap()).unwrap();
    assert!(
        ProofVerifier
            .verify(&changed, artifact.statement())
            .is_err()
    );
    let relabelled = Statement::new(
        ProofKind::Settlement,
        claim.domain(),
        preparation_image_id(),
        artifact.statement().payload().to_vec(),
    )
    .unwrap();
    let changed = ProofEnvelope::new(&relabelled, artifact.proof().proof().to_vec()).unwrap();
    assert_eq!(
        ProofVerifier.verify(&changed, &relabelled).unwrap_err(),
        Error::BackendUnavailable
    );
    if let Some(evidence) = std::env::var_os("ZUTTI_PROOF_DIR") {
        let evidence = std::path::PathBuf::from(evidence);
        std::fs::create_dir_all(&evidence).unwrap();
        artifact
            .persist_new(&evidence.join("preparation.proof"))
            .unwrap();
        let report = serde_json::json!({"seconds": elapsed, "artifact_bytes": artifact.to_bytes().unwrap().len(), "envelope_bytes": artifact.proof().to_bytes().unwrap().len(), "journal_bytes": artifact.statement().payload().len(), "image_id": preparation_image_id().to_string(), "consensus_verified": false, "settlement_authority": false});
        std::fs::write(
            evidence.join("preparation.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
    }
    eprintln!("preparation proof: {elapsed:.3} s");
}
