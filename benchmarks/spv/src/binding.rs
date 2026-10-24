use crate::{DEFAULT_CYCLES, Result, image, verify, write_new};
use bincode::Options;
use risc0_zkvm::{Executor, ExecutorEnv, ExternalProver, Prover, ProverOpts, VerifierContext};
use sha2::{Digest as _, Sha256};
use std::{fs, path::Path, time::Instant};
use zutti_protocol::encoding::Canonical;
use zutti_wallet::binding::PaymentBindingWitness;

pub fn run(
    program: &[u8],
    witness: &PaymentBindingWitness,
    out: &Path,
    prove: bool,
    negative: bool,
) -> Result<()> {
    if prove && negative {
        return Err("negative cases are execution checks".into());
    }
    let expected = witness.evaluate();
    if negative != expected.is_err() {
        return Err("native binding outcome differs from the requested case".into());
    }
    let input = witness.to_bytes()?;
    let mut builder = ExecutorEnv::builder();
    builder
        .write_slice(&(input.len() as u32).to_le_bytes())
        .write_slice(&input)
        .session_limit(Some(DEFAULT_CYCLES))
        .segment_limit_po2(20);
    fs::create_dir(out)?;
    write_new(&out.join("witness.bin"), &input)?;
    let started = Instant::now();
    let prover = ExternalProver::new("zutti-payment-binding", "r0vm");
    let image = image(program)?;
    let mut cycles = None;
    let journal = match expected {
        Ok(claim) => {
            let journal = claim.to_bytes()?;
            write_new(&out.join("journal.bin"), &journal)?;
            if prove {
                let info = prover.prove_with_ctx(
                    builder.build()?,
                    &VerifierContext::default().with_dev_mode(false),
                    program,
                    &ProverOpts::succinct().with_dev_mode(false),
                )?;
                verify(&info.receipt, image, &journal)?;
                if !matches!(info.receipt.inner, risc0_zkvm::InnerReceipt::Succinct(_)) {
                    return Err("binding prover returned an unexpected receipt kind".into());
                }
                let codec = bincode::DefaultOptions::new()
                    .with_fixint_encoding()
                    .with_limit(crate::MAX_RECEIPT as u64);
                write_new(&out.join("receipt.bin"), &codec.serialize(&info.receipt)?)?;
                cycles = Some(info.stats.user_cycles);
            } else {
                let session = prover.execute(builder.build()?, program)?;
                if session.exit_code != risc0_zkvm::ExitCode::Halted(0)
                    || session.journal.bytes != journal
                {
                    return Err("binding guest execution or journal differs".into());
                }
                cycles = Some(session.cycles());
            }
            Some(hex::encode(Sha256::digest(&journal)))
        }
        Err(_) => {
            let error = prover
                .execute(builder.build()?, program)
                .err()
                .ok_or("binding guest accepted the negative case")?;
            if !error.to_string().contains("invalid payment window binding") {
                return Err(format!("binding guest failed before its predicate: {error}").into());
            }
            None
        }
    };
    let report = serde_json::json!({"schema":1,"image_id":image.to_string(),
        "program_sha256":hex::encode(Sha256::digest(program)),"witness_sha256":hex::encode(Sha256::digest(&input)),
        "journal_sha256":journal,"native_rejected":negative,"guest_rejected":negative,
        "journal_matches":!negative,"cycles":cycles,"seconds":started.elapsed().as_secs_f64(),
        "proof_generated":prove,"proof_verified":prove,"paid_jobs":0,"source_anchor_authenticated":false,
        "lower_boundary_model":"anchor-hash dependency with binding output encryption and no feasible commitment cycle",
        "security_review_complete":false,"ethereum_wrapped":false,"settlement_authority":false});
    write_new(
        &out.join("result.json"),
        &serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{report}");
    Ok(())
}
