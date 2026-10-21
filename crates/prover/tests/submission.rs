#![allow(clippy::unwrap_used)]

#[path = "../../../tests/support/swap.rs"]
mod support;

use bincode::Options;
use risc0_zkvm::{FakeReceipt, InnerReceipt, Receipt, ReceiptClaim};
use zutti_protocol::{
    Digest,
    encoding::Canonical,
    order::Direction,
    pool::{PoolClaim, nullifier_digest},
    statement::{ProofKind, Statement},
};
use zutti_prover::{
    Error, ProofEnvelope,
    artifact::ProofArtifact,
    pool::pool_statement,
    remote::import_groth16,
    submission::{PoolExecution, PoolOperation, PublicOutput},
};
use zutti_wallet::{
    backup,
    swap::{SettlementAction, SwapRecord},
};

fn fixture(name: &str) -> serde_json::Value {
    let data: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/pool-lifecycle.json")).unwrap();
    data["operations"][name].clone()
}

fn operation(data: &serde_json::Value) -> PoolOperation {
    PoolOperation::new(
        PoolClaim::from_bytes(&hex::decode(data["journal"].as_str().unwrap()).unwrap()).unwrap(),
        data["nullifiers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| Digest::from_hex(v.as_str().unwrap()).unwrap())
            .collect(),
        data["outputs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| PublicOutput {
                commitment: Digest::from_hex(v["commitment"].as_str().unwrap()).unwrap(),
                ciphertext: hex::decode(v["ciphertext"].as_str().unwrap()).unwrap(),
            })
            .collect(),
    )
    .unwrap()
}

fn statement(data: &serde_json::Value) -> Statement {
    let payload = hex::decode(data["journal"].as_str().unwrap()).unwrap();
    let domain = PoolClaim::from_bytes(&payload).unwrap().domain;
    Statement::new(
        ProofKind::Note,
        domain,
        Digest::from_hex(data["image_id"].as_str().unwrap()).unwrap(),
        payload,
    )
    .unwrap()
}

fn artifact(data: &serde_json::Value, expected: &Statement) -> ProofArtifact {
    let result = serde_json::json!({
        "version": 1,
        "chain_id": 8453,
        "image_id": data["image_id"],
        "journal": data["journal"],
        "seal": data["seal"],
    });
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("result.json");
    backup::persist_new(&path, &serde_json::to_vec(&result).unwrap()).unwrap();
    import_groth16(&path, expected).unwrap()
}

#[test]
fn genuine_lifecycle_receipts_export_exact_contract_fields_and_refuse_overwrite() {
    for name in ["deposit", "transfer", "withdraw"] {
        let data = fixture(name);
        let expected = statement(&data);
        let artifact = artifact(&data, &expected);
        let execution =
            PoolExecution::verify(operation(&data), &expected, &artifact, None).unwrap();
        let json = execution.to_json();
        for key in [
            "kind",
            "domain",
            "image_id",
            "root",
            "checkpoint",
            "order_tag",
            "order",
            "reservation",
            "value",
            "owner",
            "nullifiers",
            "outputs",
            "seal",
        ] {
            assert_eq!(json[key], data[key], "{name} {key}");
        }
        assert_eq!(json["preparation_seal"], "");
        assert_eq!(json["proof_verified"], true);
        assert_eq!(json["chain_admission_verified"], false);
        assert_eq!(json["transaction_submitted"], false);
        for key in ["txid", "payment", "order_randomness", "nullifier_key"] {
            assert!(json.get(key).is_none());
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("execution.json");
        execution.persist_new(&path).unwrap();
        let saved = std::fs::read(&path).unwrap();
        assert!(execution.persist_new(&path).is_err());
        assert_eq!(std::fs::read(path).unwrap(), saved);
    }
}

#[test]
fn public_operation_rejects_changed_ciphertexts_nullifiers_and_shape() {
    let data = fixture("transfer");
    let mut changed = data.clone();
    changed["outputs"][0]["ciphertext"] = "01".into();
    let outputs = changed["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| PublicOutput {
            commitment: Digest::from_hex(v["commitment"].as_str().unwrap()).unwrap(),
            ciphertext: hex::decode(v["ciphertext"].as_str().unwrap()).unwrap(),
        })
        .collect();
    assert!(
        PoolOperation::new(
            operation(&data).claim().clone(),
            vec![Digest::from_hex(data["nullifiers"][0].as_str().unwrap()).unwrap()],
            outputs
        )
        .is_err()
    );
    assert!(PoolOperation::new(operation(&data).claim().clone(), vec![], vec![]).is_err());
    let expected = statement(&data);
    let artifact = artifact(&data, &expected);
    let mut claim = operation(&data).claim().clone();
    claim.nullifiers = nullifier_digest(&[Digest([44; 32])]).unwrap();
    let outputs = data["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| PublicOutput {
            commitment: Digest::from_hex(v["commitment"].as_str().unwrap()).unwrap(),
            ciphertext: hex::decode(v["ciphertext"].as_str().unwrap()).unwrap(),
        })
        .collect();
    let changed_operation =
        PoolOperation::new(claim.clone(), vec![Digest([44; 32])], outputs).unwrap();
    let changed_expected = Statement::new(
        ProofKind::Note,
        claim.domain,
        expected.program(),
        claim.to_bytes().unwrap(),
    )
    .unwrap();
    assert!(PoolExecution::verify(changed_operation, &changed_expected, &artifact, None).is_err());
}

#[test]
fn fake_receipts_wrong_profiles_and_unexpected_preparation_never_export() {
    let data = fixture("deposit");
    let expected = statement(&data);
    let real = artifact(&data, &expected);
    let receipt = Receipt::new(
        InnerReceipt::Fake(FakeReceipt::new(ReceiptClaim::ok(
            expected.program().0,
            expected.payload().to_vec(),
        ))),
        expected.payload().to_vec(),
    );
    let bytes = bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .serialize(&receipt)
        .unwrap();
    let fake = ProofArtifact::new(
        expected.clone(),
        ProofEnvelope::new(&expected, bytes).unwrap(),
    )
    .unwrap();
    assert_eq!(
        PoolExecution::verify(operation(&data), &expected, &fake, None).unwrap_err(),
        Error::DevelopmentReceipt
    );
    let wrong = Statement::new(
        ProofKind::Note,
        expected.domain(),
        Digest([99; 32]),
        expected.payload().to_vec(),
    )
    .unwrap();
    assert!(PoolExecution::verify(operation(&data), &wrong, &real, None).is_err());
    assert!(
        PoolExecution::verify(operation(&data), &expected, &real, Some((&expected, &real)))
            .is_err()
    );
}

#[test]
fn reservation_requires_preparation_and_settlement_builds_one_fixed_output() {
    let data = fixture("deposit");
    let artifact = artifact(&data, &statement(&data));
    let (reserve, evidence, _, _) = support::scenario(true, Direction::ZecToUsdc, 2);
    let operation = PoolOperation::from_witness(&reserve).unwrap();
    let expected = pool_statement(operation.claim()).unwrap();
    assert_eq!(
        PoolExecution::verify(operation, &expected, &artifact, None).unwrap_err(),
        Error::Binding
    );
    let record = SwapRecord::from_reservation(&reserve).unwrap();
    let witness = record
        .settlement(SettlementAction::Claim, &evidence)
        .unwrap();
    let operation = PoolOperation::from_witness(&witness).unwrap();
    assert_eq!(operation.claim(), &witness.claim);
    let expected = pool_statement(operation.claim()).unwrap();
    assert!(PoolExecution::verify(operation, &expected, &artifact, None).is_err());
}
