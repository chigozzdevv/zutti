#![allow(clippy::unwrap_used)]

use zutti_protocol::{
    Digest,
    encoding::Canonical,
    statement::{ProofKind, Statement},
};
use zutti_prover::remote::import_groth16;
use zutti_wallet::{backup, preparation::PreparationClaim};

fn statement(value: &serde_json::Value, kind: ProofKind, program: Digest) -> Statement {
    let payload = hex::decode(value["journal"].as_str().unwrap()).unwrap();
    let claim = PreparationClaim::from_bytes(&payload).unwrap();
    Statement::new(kind, claim.domain(), program, payload).unwrap()
}

#[test]
fn pinned_preparation_receipt_survives_native_crate_changes() {
    let value: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/preparation-proof.json"
    ))
    .unwrap();
    let expected = statement(
        &value,
        ProofKind::Preparation,
        Digest::from_hex(value["image_id"].as_str().unwrap()).unwrap(),
    );
    let directory = tempfile::tempdir().unwrap();
    let result = directory.path().join("result.json");
    backup::persist_new(&result, &serde_json::to_vec(&value).unwrap()).unwrap();
    let artifact = import_groth16(&result, &expected).unwrap();
    artifact.verify(&expected).unwrap();
    for kind in [
        ProofKind::Source,
        ProofKind::HistoryQuery,
        ProofKind::Settlement,
    ] {
        assert!(import_groth16(&result, &statement(&value, kind, expected.program())).is_err());
    }
    let mut changed = value;
    let mut seal = hex::decode(changed["seal"].as_str().unwrap()).unwrap();
    seal[259] ^= 1;
    changed["seal"] = hex::encode(seal).into();
    let result = directory.path().join("changed.json");
    backup::persist_new(&result, &serde_json::to_vec(&changed).unwrap()).unwrap();
    assert!(import_groth16(&result, &expected).is_err());
}

#[test]
fn otherwise_valid_receipt_cannot_admit_an_unknown_program() {
    let mut value: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/preparation-proof.json"
    ))
    .unwrap();
    let image = Digest([9; 32]);
    value["image_id"] = image.to_string().into();
    let expected = statement(&value, ProofKind::Preparation, image);
    let directory = tempfile::tempdir().unwrap();
    let result = directory.path().join("result.json");
    backup::persist_new(&result, &serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(import_groth16(&result, &expected).is_err());
}
