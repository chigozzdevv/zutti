#![allow(clippy::unwrap_used)]

use bincode::Options;
use risc0_zkvm::{FakeReceipt, InnerReceipt, Receipt, ReceiptClaim};
use zutti_protocol::{
    Digest,
    encoding::Canonical,
    pool::PoolClaim,
    statement::{ProofKind, Statement},
};
use zutti_prover::{Error, ProofEnvelope, ProofVerifier, remote::import_groth16};

#[test]
fn genuine_pool_lifecycle_receipts_verify_and_reject_relabelling() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/pool-lifecycle.json")).unwrap();
    for name in ["deposit", "transfer", "withdraw"] {
        let value = &fixture["operations"][name];
        let payload = hex::decode(value["journal"].as_str().unwrap()).unwrap();
        let claim = PoolClaim::from_bytes(&payload).unwrap();
        let image = Digest::from_hex(value["image_id"].as_str().unwrap()).unwrap();
        let statement =
            Statement::new(ProofKind::Note, claim.domain, image, payload.clone()).unwrap();
        let result = serde_json::json!({"version":1,"chain_id":8453,"image_id":image.to_string(),"journal":value["journal"],"seal":value["seal"]});
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("result.json");
        zutti_wallet::backup::persist_new(&path, &serde_json::to_vec(&result).unwrap()).unwrap();
        import_groth16(&path, &statement)
            .unwrap()
            .verify(&statement)
            .unwrap();
        for kind in [
            ProofKind::Reservation,
            ProofKind::Settlement,
            ProofKind::Source,
        ] {
            let wrong = Statement::new(kind, claim.domain, image, payload.clone()).unwrap();
            assert!(import_groth16(&path, &wrong).is_err());
        }
        let mut changed = result;
        let mut seal = hex::decode(changed["seal"].as_str().unwrap()).unwrap();
        seal[259] ^= 1;
        changed["seal"] = hex::encode(seal).into();
        let path = directory.path().join("changed.json");
        zutti_wallet::backup::persist_new(&path, &serde_json::to_vec(&changed).unwrap()).unwrap();
        assert!(import_groth16(&path, &statement).is_err());
    }
}

#[test]
fn exact_pool_program_and_journal_never_admit_development_receipts() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/pool-lifecycle.json")).unwrap();
    let payload = hex::decode(
        fixture["operations"]["deposit"]["journal"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    let claim = PoolClaim::from_bytes(&payload).unwrap();
    let statement = zutti_prover::pool::pool_statement(&claim).unwrap();
    let receipt = Receipt::new(
        InnerReceipt::Fake(FakeReceipt::new(ReceiptClaim::ok(
            statement.program().0,
            payload.clone(),
        ))),
        payload,
    );
    let bytes = bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .serialize(&receipt)
        .unwrap();
    let proof = ProofEnvelope::new(&statement, bytes).unwrap();
    assert_eq!(
        ProofVerifier.verify(&proof, &statement).unwrap_err(),
        Error::DevelopmentReceipt
    );
}
