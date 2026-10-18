#![allow(clippy::unwrap_used)]

use zcash_protocol::consensus::NetworkType;
use zutti_protocol::{
    Digest,
    encoding::Canonical,
    statement::{ProofKind, Statement},
};
use zutti_prover::{artifact::ProofArtifact, remote::import_groth16};
use zutti_source::proof::{SourceInput, SourcePolicy, SourceReceiptClaim, SourceWitness};

fn import(value: &serde_json::Value, kind: ProofKind, domain: Digest) -> ProofArtifact {
    let payload = hex::decode(value["journal"].as_str().unwrap()).unwrap();
    let program = Digest::from_hex(value["image_id"].as_str().unwrap()).unwrap();
    let expected = Statement::new(kind, domain, program, payload).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("result.json");
    zutti_wallet::backup::persist_new(&path, &serde_json::to_vec(value).unwrap()).unwrap();
    import_groth16(&path, &expected).unwrap()
}

fn genesis() -> (serde_json::Value, SourceReceiptClaim, SourcePolicy) {
    let value: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/source-optimized-genesis-proof.json"
    ))
    .unwrap();
    let claim =
        SourceReceiptClaim::from_bytes(&hex::decode(value["journal"].as_str().unwrap()).unwrap())
            .unwrap();
    let policy = SourcePolicy {
        program: claim.program,
        registry_program: Digest::from_hex(
            "6d89225b110c7548c11382263ba1941cb8b6d28c49d2a8c087cbbdc0694d7411",
        )
        .unwrap(),
        network: NetworkType::Test,
        index_start: 0,
        confirmations: 3,
    };
    (value, claim, policy)
}

#[test]
fn optimized_genesis_receipt_matches_native_validation_and_rejects_tampering() {
    let (value, claim, policy) = genesis();
    let proof = import(&value, ProofKind::SourceTransition, claim.policy);
    proof.verify(proof.statement()).unwrap();
    let expected = SourceWitness {
        policy,
        input: SourceInput::Genesis(zebra_test::vectors::TESTNET_BLOCKS[&0].to_vec()),
    }
    .evaluate()
    .unwrap();
    assert_eq!(claim, expected);
    let mut changed = value;
    let mut seal = hex::decode(changed["seal"].as_str().unwrap()).unwrap();
    seal[259] ^= 1;
    changed["seal"] = hex::encode(seal).into();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("result.json");
    zutti_wallet::backup::persist_new(&path, &serde_json::to_vec(&changed).unwrap()).unwrap();
    assert!(import_groth16(&path, proof.statement()).is_err());
}
