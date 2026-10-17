#![allow(clippy::unwrap_used)]
#[path = "../../../tests/support/payment.rs"]
mod payment;
#[path = "../../../tests/support/mod.rs"]
mod support;

use risc0_zkvm::{Groth16ReceiptVerifierParameters, sha::Digestible};
use zutti_prover::remote::{export_public_preparation, import_groth16};
use zutti_wallet::preparation::PreparationWitness;

#[test]
fn public_job_binds_program_input_and_expected_claim() {
    let fixture = payment::fixture(false);
    let witness = PreparationWitness::new(
        fixture.terms,
        fixture.randomness,
        support::secret(32),
        fixture.pczt.serialize().unwrap(),
    )
    .unwrap();
    let parent = tempfile::tempdir().unwrap();
    let directory = parent.path().join("public");
    let statement = export_public_preparation(&witness, &directory).unwrap();
    let job: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("job.json")).unwrap()).unwrap();
    assert_eq!(job["visibility"], "public");
    assert_eq!(job["image_id"], statement.program().to_string());
    assert_eq!(
        std::fs::read(directory.join("journal.bin")).unwrap(),
        statement.payload()
    );
    assert!(export_public_preparation(&witness, &directory).is_err());
    let mut seal = vec![0_u8; 260];
    let parameters = Groth16ReceiptVerifierParameters::default().digest();
    seal[..4].copy_from_slice(&parameters.as_bytes()[..4]);
    let result = directory.join("result.json");
    let value = serde_json::json!({"version": 1, "chain_id": 8453, "image_id": statement.program().to_string(), "journal": hex::encode(statement.payload()), "seal": hex::encode(&seal)});
    zutti_wallet::backup::persist_new(&result, &serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(import_groth16(&result, &statement).is_err());
}
