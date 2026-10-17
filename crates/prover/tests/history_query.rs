#![allow(clippy::unwrap_used)]
#[path = "../../../tests/support/mod.rs"]
mod support;
use bincode::Options;
use risc0_zkvm::{FakeReceipt, InnerReceipt, Receipt, ReceiptClaim};
use zcash_protocol::consensus::MAIN_NETWORK;
use zutti_protocol::{
    Digest,
    encoding::Canonical,
    order::OrderTerms,
    policy::PaymentWindow,
    statement::{ProofKind, Statement},
};
use zutti_prover::{
    Error, ProofEnvelope, ProofVerifier,
    artifact::ProofArtifact,
    query::{LocalProver, query_image_id, query_statement},
};
use zutti_source::{
    CandidateIndex, IndexContext, block::ParsedBlock, consensus::verify_history,
    query::QueryWitness,
};

fn scenario(paid: bool) -> QueryWitness {
    let blocks: Vec<_> = [
        include_str!("../../../tests/fixtures/block-1687106.txt"),
        include_str!("../../../tests/fixtures/block-1687107.txt"),
        include_str!("../../../tests/fixtures/block-1687108.txt"),
    ]
    .iter()
    .map(|text| ParsedBlock::parse(&hex::decode(text.trim()).unwrap(), &MAIN_NETWORK).unwrap())
    .collect();
    let mut input = support::input();
    input.window = PaymentWindow::new(1_687_105, 1_687_106, blocks[0].hash(), 1_687_107).unwrap();
    let mut tree = CandidateIndex::new(
        IndexContext::new(
            input.domain.source_genesis(),
            input.domain.source_policy(),
            input.window.index_start(),
        )
        .unwrap(),
    );
    for block in &blocks {
        tree.append_block(block.claimed_height(), block.txids())
            .unwrap();
    }
    let txid = if paid {
        *blocks[1].txids().last().unwrap()
    } else {
        Digest([90; 32])
    };
    QueryWitness::new(
        OrderTerms::new(input).unwrap(),
        support::secret(31),
        support::secret(32),
        txid,
        tree.prove(txid),
        tree.snapshot(),
    )
}

fn codec() -> impl Options {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .reject_trailing_bytes()
}

#[test]
fn fake_receipts_are_rejected_even_for_the_exact_image_and_journal() {
    let claim = scenario(true).evaluate().unwrap();
    let statement = query_statement(&claim).unwrap();
    let receipt = Receipt::new(
        InnerReceipt::Fake(FakeReceipt::new(ReceiptClaim::ok(
            zutti_methods::HISTORY_QUERY_ID,
            statement.payload().to_vec(),
        ))),
        statement.payload().to_vec(),
    );
    let envelope = ProofEnvelope::new(&statement, codec().serialize(&receipt).unwrap()).unwrap();
    assert_eq!(
        ProofVerifier.verify(&envelope, &statement).unwrap_err(),
        Error::DevelopmentReceipt
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("fake.proof");
    ProofArtifact::new(statement, envelope)
        .unwrap()
        .persist_new(&path)
        .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_private-prover"))
        .args(["verify-query", "--proof"])
        .arg(&path)
        .env("RISC0_DEV_MODE", "1")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("development receipts")
    );
}

#[test]
fn malformed_receipts_and_unregistered_programs_are_rejected() {
    let claim = scenario(true).evaluate().unwrap();
    let statement = query_statement(&claim).unwrap();
    for bytes in [
        vec![0; 64],
        vec![255; 128],
        vec![0; 4],
        vec![3, 0, 0, 0, 255, 255, 255, 255, 255, 255, 255, 255],
    ] {
        let envelope = ProofEnvelope::new(&statement, bytes).unwrap();
        assert_eq!(
            ProofVerifier.verify(&envelope, &statement).unwrap_err(),
            Error::Receipt
        );
    }
    let unregistered = Statement::new(
        ProofKind::HistoryQuery,
        claim.domain(),
        Digest([99; 32]),
        claim.to_bytes().unwrap(),
    )
    .unwrap();
    assert_eq!(
        ProofVerifier
            .verify(
                &ProofEnvelope::new(&unregistered, vec![1]).unwrap(),
                &unregistered
            )
            .unwrap_err(),
        Error::BackendUnavailable
    );
}

#[test]
fn query_statements_bind_the_local_program_and_history_claim() {
    let paid = scenario(true).evaluate().unwrap();
    let absent = scenario(false).evaluate().unwrap();
    assert_eq!(paid.output(), support::terms().payout().commitment());
    assert_eq!(absent.output(), support::terms().refund().commitment());
    assert_eq!(paid.snapshot(), absent.snapshot());
    assert_eq!(paid.order(), absent.order());
    assert_ne!(paid.reservation(), absent.reservation());
    let statement = query_statement(&paid).unwrap();
    assert_eq!(statement.program(), query_image_id());
    assert_eq!(statement.kind(), ProofKind::HistoryQuery);
    assert_eq!(statement.domain(), paid.domain());
    assert_eq!(statement.payload(), paid.to_bytes().unwrap());
    assert_eq!(
        paid.to_bytes().unwrap().len(),
        absent.to_bytes().unwrap().len()
    );
}

#[test]
fn proof_artifacts_are_bounded_bound_to_the_statement_and_never_overwrite() {
    let statement = query_statement(&scenario(true).evaluate().unwrap()).unwrap();
    let envelope = ProofEnvelope::new(&statement, vec![1; 64]).unwrap();
    let changed = query_statement(&scenario(false).evaluate().unwrap()).unwrap();
    assert!(ProofArtifact::new(changed, envelope.clone()).is_err());
    let artifact = ProofArtifact::new(statement, envelope).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("query.proof");
    artifact.persist_new(&path).unwrap();
    assert!(artifact.persist_new(&path).is_err());
    let decoded = ProofArtifact::read(&path).unwrap();
    assert_eq!(decoded.statement(), artifact.statement());
    assert_eq!(decoded.proof(), artifact.proof());
    assert!(decoded.verify(artifact.statement()).is_err());
    let mut encoded = artifact.to_bytes().unwrap();
    for end in [0, 8, 64, encoded.len() - 1] {
        assert!(ProofArtifact::from_bytes(&encoded[..end]).is_err());
    }
    encoded.push(0);
    assert!(ProofArtifact::from_bytes(&encoded).is_err());
}

fn verify_real_case(paid: bool) {
    let tools = tempfile::tempdir().unwrap();
    let cli = tools.path().join("private-prover");
    std::fs::copy(env!("CARGO_BIN_EXE_private-prover"), &cli).unwrap();
    let status = std::process::Command::new(&cli)
        .arg("status")
        .output()
        .unwrap();
    assert!(status.status.success());
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["image_id"], query_image_id().to_string());
    let witness = scenario(paid);
    let claim = witness.evaluate().unwrap();
    let started = std::time::Instant::now();
    let executable = std::env::var_os("RISC0_SERVER_PATH").unwrap_or_else(|| "r0vm".into());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("query.proof");
    let (statement, proof) = if paid {
        LocalProver::new(&executable).prove_query(&witness).unwrap()
    } else {
        let key_file = directory.path().join("recovery.key");
        let archive = directory.path().join("query.backup");
        let key = support::secret(61);
        zutti_wallet::backup::persist_new(&key_file, key.expose()).unwrap();
        let plaintext = zeroize::Zeroizing::new(witness.to_bytes().unwrap());
        let encrypted = zutti_wallet::backup::seal(claim.domain(), &plaintext, &key).unwrap();
        zutti_wallet::backup::persist_new(&archive, &encrypted).unwrap();
        let output = std::process::Command::new(&cli)
            .args(["prove-query", "--backup"])
            .arg(&archive)
            .arg("--key-file")
            .arg(&key_file)
            .arg("--domain")
            .arg(claim.domain().to_string())
            .arg("--out")
            .arg(&path)
            .arg("--prover")
            .arg(&executable)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let status: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(status["proof_verified"], true);
        assert_eq!(status["settlement_authority"], false);
        assert!(status.get("outcome").is_none());
        let artifact = ProofArtifact::read(&path).unwrap();
        (artifact.statement().clone(), artifact.proof().clone())
    };
    let elapsed = started.elapsed();
    let verified = ProofVerifier.verify(&proof, &statement).unwrap();
    assert_eq!(verified.kind(), ProofKind::HistoryQuery);
    assert_eq!(verified.statement(), statement.digest().unwrap());
    assert!(verify_history(claim.snapshot(), &proof.to_bytes().unwrap()).is_err());
    if paid {
        ProofArtifact::new(statement.clone(), proof.clone())
            .unwrap()
            .persist_new(&path)
            .unwrap();
    }
    let output = std::process::Command::new(&cli)
        .args(["verify-query", "--proof"])
        .arg(&path)
        .env("RISC0_DEV_MODE", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let status: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(status["proof_verified"], true);
    assert_eq!(status["consensus_verified"], false);
    assert!(status.get("outcome").is_none());
    let receipt: Receipt = codec().deserialize(proof.proof()).unwrap();
    assert!(matches!(receipt.inner, InnerReceipt::Succinct(_)));
    assert_eq!(receipt.journal.bytes, claim.to_bytes().unwrap());
    let changed = query_statement(&scenario(!paid).evaluate().unwrap()).unwrap();
    assert!(ProofVerifier.verify(&proof, &changed).is_err());
    let relabelled = ProofEnvelope::new(&changed, proof.proof().to_vec()).unwrap();
    assert!(ProofVerifier.verify(&relabelled, &changed).is_err());
    for kind in [
        ProofKind::Source,
        ProofKind::Preparation,
        ProofKind::Note,
        ProofKind::Reservation,
        ProofKind::Settlement,
    ] {
        let other = Statement::new(
            kind,
            statement.domain(),
            statement.program(),
            statement.payload().to_vec(),
        )
        .unwrap();
        let relabelled = ProofEnvelope::new(&other, proof.proof().to_vec()).unwrap();
        assert_eq!(
            ProofVerifier.verify(&relabelled, &other).unwrap_err(),
            Error::BackendUnavailable
        );
    }
    let mut tampered = receipt;
    tampered.journal.bytes[11] ^= 1;
    let tampered = ProofEnvelope::new(&statement, codec().serialize(&tampered).unwrap()).unwrap();
    assert!(ProofVerifier.verify(&tampered, &statement).is_err());
    let mut trailing = proof.proof().to_vec();
    trailing.push(0);
    assert!(
        ProofVerifier
            .verify(
                &ProofEnvelope::new(&statement, trailing).unwrap(),
                &statement
            )
            .is_err()
    );
    let name = if paid { "paid" } else { "expired_absent" };
    let metrics = serde_json::json!({"case": name, "seconds": elapsed.as_secs_f64(), "proof_bytes": proof.to_bytes().unwrap().len(), "image_id": query_image_id().to_string(), "consensus_verified": false});
    if let Some(output) = std::env::var_os("ZUTTI_PROOF_DIR") {
        let output = std::path::PathBuf::from(output);
        ProofArtifact::new(statement, proof)
            .unwrap()
            .persist_new(&output.join(format!("{name}.proof")))
            .unwrap();
        std::fs::write(
            output.join(format!("{name}.json")),
            serde_json::to_vec_pretty(&metrics).unwrap(),
        )
        .unwrap();
    }
    println!("{metrics}");
}

#[test]
#[ignore = "requires local r0vm and generates a cryptographic receipt"]
fn real_paid_proof_verifies_and_tampering_fails() {
    verify_real_case(true);
}

#[test]
#[ignore = "requires local r0vm and generates a cryptographic receipt"]
fn real_absent_proof_verifies_and_tampering_fails() {
    verify_real_case(false);
}

#[test]
#[ignore = "measures execution of the history-query guest"]
fn history_query_execution_cost() {
    use risc0_zkvm::{Executor, ExecutorEnv, ExternalProver};
    let witness = scenario(true);
    let bytes = zeroize::Zeroizing::new(witness.to_bytes().unwrap());
    let length = u32::try_from(bytes.len()).unwrap().to_le_bytes();
    let env = ExecutorEnv::builder()
        .write_slice(&length)
        .write_slice(bytes.as_slice())
        .session_limit(Some(16_777_216))
        .segment_limit_po2(18)
        .build()
        .unwrap();
    let executable = std::env::var_os("RISC0_SERVER_PATH").unwrap_or_else(|| "r0vm".into());
    let session = ExternalProver::new("zutti-measure", executable)
        .execute(env, zutti_methods::HISTORY_QUERY_ELF)
        .unwrap();
    assert_eq!(
        session.journal.bytes,
        witness.evaluate().unwrap().to_bytes().unwrap()
    );
    eprintln!(
        "history query execution: {} cycles, {} segments",
        session.cycles(),
        session.segments.len()
    );
}
