use bincode::Options;
use risc0_zkvm::{
    Executor, ExecutorEnv, ExternalProver, Prover, ProverOpts, Receipt, ReceiptClaim,
    VerifierContext,
};
use sha2::{Digest as _, Sha256};
use std::{fs, io::Read, path::Path, time::Instant};
use zutti_protocol::{Digest, encoding::Canonical};
use zutti_source::spv::{HeaderClaim, MAX_WITNESS, Witness};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
pub const MAX_RECEIPT: usize = 16 * 1024 * 1024;
pub const DEFAULT_CYCLES: u64 = 1 << 28;
pub const MAX_CYCLES: u64 = 1 << 38;

#[derive(Debug, Clone, Copy)]
pub struct RunOptions {
    pub prove: bool,
    pub cycle_limit: u64,
    pub public_header_receipt: bool,
}

pub fn bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let file = fs::File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err("input must be a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() > limit {
        return Err("input exceeds its bounds".into());
    }
    Ok(bytes)
}

pub fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn codec() -> impl Options {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(MAX_RECEIPT as u64)
        .reject_trailing_bytes()
}

pub fn read_receipt(path: &Path) -> Result<Receipt> {
    Ok(codec().deserialize(&bounded(path, MAX_RECEIPT)?)?)
}

pub fn image(program: &[u8]) -> Result<Digest> {
    Ok(Digest(risc0_binfmt::compute_image_id(program)?.into()))
}

pub fn verify(receipt: &Receipt, image: Digest, journal: &[u8]) -> Result<()> {
    receipt.verify_with_context(&VerifierContext::default().with_dev_mode(false), image.0)?;
    if receipt.journal.bytes != journal {
        return Err("receipt journal mismatch".into());
    }
    Ok(())
}

pub fn check_previous(witness: &Witness, receipt: Option<&Receipt>, image: Digest) -> Result<()> {
    if witness.policy().program() != image {
        return Err("SPV program does not match its policy".into());
    }
    match (witness.assumption(), receipt) {
        (Some(previous), Some(receipt)) => verify(receipt, image, &previous.to_bytes()?),
        (Some(_), None) => Err("genuine predecessor evidence is required".into()),
        (None, None) => Ok(()),
        (None, Some(_)) => Err("genesis cannot consume a predecessor".into()),
    }
}

pub fn run(
    program: &[u8],
    witness: &Witness,
    previous: Option<&Receipt>,
    out: &Path,
    options: RunOptions,
) -> Result<Option<Receipt>> {
    let RunOptions {
        prove,
        cycle_limit,
        public_header_receipt,
    } = options;
    if public_header_receipt && (!prove || matches!(witness, Witness::Window { .. })) {
        return Err("composite mode is restricted to public header proving".into());
    }
    if out.try_exists()? || cycle_limit == 0 || cycle_limit > MAX_CYCLES {
        return Err("output exists or cycle limit is invalid".into());
    }
    let image_id = image(program)?;
    if witness.policy().program() != image_id {
        return Err("SPV program mismatch".into());
    }
    if prove || previous.is_some() {
        check_previous(witness, previous, image_id)?;
    }
    let input = witness.to_bytes()?;
    if input.len() > MAX_WITNESS {
        return Err("SPV input exceeds its bound".into());
    }
    let journal = witness.evaluate()?.journal()?;
    fs::create_dir(out)?;
    write_new(&out.join("witness.bin"), &input)?;
    write_new(&out.join("journal.bin"), &journal)?;
    let mut stdin = (input.len() as u32).to_le_bytes().to_vec();
    stdin.extend_from_slice(&input);
    let mut builder = ExecutorEnv::builder();
    builder
        .write_slice(&stdin)
        .session_limit(Some(cycle_limit))
        .segment_limit_po2(20);
    if let Some(receipt) = previous {
        builder.add_assumption(receipt.clone());
    } else if let Some(previous) = witness.assumption() {
        builder.add_assumption(ReceiptClaim::ok(image_id.0, previous.to_bytes()?));
    }
    let started = Instant::now();
    let prover = ExternalProver::new("zutti-spv-prototype", "r0vm");
    let (receipt, cycles, segments) = if prove {
        let result = prover.prove_with_ctx(
            builder.build()?,
            &VerifierContext::default().with_dev_mode(false),
            program,
            &if public_header_receipt {
                ProverOpts::composite()
            } else {
                ProverOpts::succinct()
            }
            .with_dev_mode(false),
        )?;
        verify(&result.receipt, image_id, &journal)?;
        if !matches!(
            (public_header_receipt, &result.receipt.inner),
            (true, risc0_zkvm::InnerReceipt::Composite(_))
                | (false, risc0_zkvm::InnerReceipt::Succinct(_))
        ) {
            return Err("prover returned an unexpected receipt format".into());
        }
        let encoded = codec().serialize(&result.receipt)?;
        write_new(&out.join("receipt.bin"), &encoded)?;
        (
            Some(result.receipt),
            result.stats.user_cycles,
            result.stats.segments,
        )
    } else {
        let result = prover.execute(builder.build()?, program)?;
        if result.exit_code != risc0_zkvm::ExitCode::Halted(0) || result.journal.bytes != journal {
            return Err("SPV execution failed or journal mismatch".into());
        }
        (None, result.cycles(), result.segments.len())
    };
    let report = serde_json::json!({
        "schema":1,"image_id":image_id.to_string(),"program_sha256":hex::encode(Sha256::digest(program)),
        "witness_sha256":hex::encode(Sha256::digest(&input)),"journal_sha256":hex::encode(Sha256::digest(&journal)),
        "cycles":cycles,"segments":segments,"seconds":started.elapsed().as_secs_f64(),
        "journal_matches":true,"proof_generated":prove,"proof_verified":prove,
        "receipt_kind":if prove {Some(if public_header_receipt {"composite"} else {"succinct"})} else {None},
        "predecessor_proof_verified":previous.is_some(),"unresolved_execution_assumption":!prove && witness.assumption().is_some() && previous.is_none(),
        "full_transaction_validity_verified":false,"canonicality_admitted":false,
        "payment_preparation_verified":false,"lower_boundary_birth_verified":false,
        "settlement_authority":false,"paid_jobs":0,
    });
    write_new(
        &out.join("result.json"),
        &serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{report}");
    Ok(receipt)
}

pub fn verify_header_receipt(program: &[u8], receipt: &Receipt) -> Result<HeaderClaim> {
    let claim = HeaderClaim::from_bytes(&receipt.journal.bytes)?;
    if claim.policy().program() != image(program)? {
        return Err("SPV prefix program mismatch".into());
    }
    verify(receipt, image(program)?, &claim.to_bytes()?)?;
    Ok(claim)
}

pub fn reject(
    program: &[u8],
    witness: &Witness,
    previous: Option<&Receipt>,
    out: &Path,
) -> Result<()> {
    if out.try_exists()? || witness.policy().program() != image(program)? {
        return Err("rejection output exists or program differs".into());
    }
    if previous.is_some() {
        check_previous(witness, previous, image(program)?)?;
    }
    let native_error = witness
        .evaluate()
        .err()
        .ok_or("native verifier accepted the negative case")?;
    let input = witness.to_bytes()?;
    let mut stdin = (input.len() as u32).to_le_bytes().to_vec();
    stdin.extend_from_slice(&input);
    let mut builder = ExecutorEnv::builder();
    builder
        .write_slice(&stdin)
        .session_limit(Some(DEFAULT_CYCLES))
        .segment_limit_po2(20);
    if let Some(receipt) = previous {
        builder.add_assumption(receipt.clone());
    } else if let Some(previous) = witness.assumption() {
        builder.add_assumption(ReceiptClaim::ok(image(program)?.0, previous.to_bytes()?));
    }
    let started = Instant::now();
    let error = ExternalProver::new("zutti-spv-rejection", "r0vm")
        .execute(builder.build()?, program)
        .err()
        .ok_or("SPV guest accepted the negative case")?;
    if !error.to_string().contains("invalid SPV source transition") {
        return Err(format!("guest failed before its SPV predicate: {error}").into());
    }
    fs::create_dir(out)?;
    write_new(&out.join("witness.bin"), &input)?;
    let report = serde_json::json!({"native_rejected":true,"guest_rejected":true,
        "native_error":native_error.to_string(),"image_id":image(program)?.to_string(),
        "witness_sha256":hex::encode(Sha256::digest(&input)),"seconds":started.elapsed().as_secs_f64(),
        "proof_generated":false,"predecessor_proof_verified":previous.is_some(),
        "settlement_authority":false,"paid_jobs":0});
    write_new(
        &out.join("result.json"),
        &serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{report}");
    Ok(())
}

pub fn audit(program: &[u8], receipt: &Receipt) -> Result<serde_json::Value> {
    let image = image(program)?;
    verify(receipt, image, &receipt.journal.bytes)?;
    let mut wrong_image = image;
    wrong_image.0[0] ^= 1;
    if verify(receipt, wrong_image, &receipt.journal.bytes).is_ok() {
        return Err("wrong program was accepted".into());
    }
    let mut journal = receipt.journal.bytes.clone();
    let first = journal.first_mut().ok_or("empty SPV journal")?;
    *first ^= 1;
    if verify(receipt, image, &journal).is_ok() {
        return Err("wrong expected journal was accepted".into());
    }
    let mut changed = receipt.clone();
    changed.journal.bytes = journal;
    if verify(&changed, image, &changed.journal.bytes).is_ok() {
        return Err("tampered receipt journal was accepted".into());
    }
    let mut changed = receipt.clone();
    let seal = match &mut changed.inner {
        risc0_zkvm::InnerReceipt::Succinct(inner) => &mut inner.seal,
        risc0_zkvm::InnerReceipt::Composite(inner) => {
            &mut inner
                .segments
                .first_mut()
                .ok_or("empty composite receipt")?
                .seal
        }
        _ => return Err("audit expects a succinct or composite receipt".into()),
    };
    let first = seal.first_mut().ok_or("empty proof seal")?;
    *first ^= 1;
    if verify(&changed, image, &receipt.journal.bytes).is_ok() {
        return Err("tampered proof seal was accepted".into());
    }
    Ok(
        serde_json::json!({"valid_receipt_verified":true,"wrong_image_rejected":true,
        "wrong_expected_journal_rejected":true,"changed_receipt_journal_rejected":true,
        "changed_proof_rejected":true,"development_mode":false,"settlement_authority":false}),
    )
}
