use crate::{Error, ProofEnvelope, artifact::ProofArtifact, backend::receipt_codec};
use bincode::Options;
use risc0_zkvm::{
    Groth16Receipt, Groth16ReceiptVerifierParameters, InnerReceipt, MaybePruned, Receipt,
    ReceiptClaim, sha::Digestible,
};
use std::path::Path;
use zeroize::Zeroizing;
use zutti_protocol::{encoding::Canonical, statement::Statement};
use zutti_wallet::{backup, preparation::PreparationWitness};

pub fn export_public_preparation(
    witness: &PreparationWitness,
    directory: &Path,
) -> Result<Statement, Box<dyn std::error::Error>> {
    std::fs::create_dir(directory)?;
    let directory = directory.canonicalize()?;
    let claim = witness.evaluate()?;
    let statement = crate::preparation::preparation_statement(&claim)?;
    let witness = Zeroizing::new(witness.to_bytes()?);
    let mut stdin = Zeroizing::new(Vec::with_capacity(witness.len() + 4));
    stdin.extend_from_slice(&u32::try_from(witness.len())?.to_le_bytes());
    stdin.extend_from_slice(&witness);
    backup::persist_new(
        &directory.join("program.elf"),
        zutti_methods::PREPARATION_ELF,
    )?;
    backup::persist_new(&directory.join("stdin.bin"), &stdin)?;
    backup::persist_new(&directory.join("journal.bin"), statement.payload())?;
    backup::persist_new(&directory.join("statement.bin"), &statement.to_bytes()?)?;
    backup::persist_new(
        &directory.join("claim.json"),
        &serde_json::to_vec(&serde_json::json!({
            "domain": claim.domain().to_string(),
            "order": claim.order().to_string(),
            "reservation": claim.reservation().to_string()
        }))?,
    )?;
    let job = serde_json::json!({
        "version": 1,
        "visibility": "public",
        "image_id": statement.program().to_string(),
        "program": directory.join("program.elf"),
        "stdin": directory.join("stdin.bin"),
        "journal": directory.join("journal.bin"),
        "cycles": 70_000_000
    });
    backup::persist_new(
        &directory.join("job.json"),
        &serde_json::to_vec_pretty(&job)?,
    )?;
    Ok(statement)
}

pub fn import_groth16(
    result: &Path,
    expected: &Statement,
) -> Result<ProofArtifact, Box<dyn std::error::Error>> {
    let bytes = backup::read_private(result, 200_000)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    if value["version"] != 1
        || value["chain_id"] != 8453
        || value["image_id"].as_str() != Some(&expected.program().to_string())
        || value["journal"].as_str() != Some(&hex::encode(expected.payload()))
    {
        return Err(Error::Binding.into());
    }
    let seal = hex::decode(value["seal"].as_str().ok_or(Error::Receipt)?)?;
    let parameters = Groth16ReceiptVerifierParameters::default().digest();
    if seal.len() != 260 || seal[..4] != parameters.as_bytes()[..4] {
        return Err(Error::Receipt.into());
    }
    let claim = ReceiptClaim::ok(
        risc0_zkvm::sha::Digest::from(expected.program().0),
        expected.payload().to_vec(),
    );
    let receipt = Receipt::new(
        InnerReceipt::Groth16(Groth16Receipt::new(
            seal[4..].to_vec(),
            MaybePruned::Value(claim),
            parameters,
        )),
        expected.payload().to_vec(),
    );
    let proof = ProofEnvelope::new(expected, receipt_codec().serialize(&receipt)?)?;
    let artifact = ProofArtifact::new(expected.clone(), proof)?;
    artifact.verify(expected)?;
    Ok(artifact)
}
