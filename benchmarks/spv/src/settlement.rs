use crate::{DEFAULT_CYCLES, Result, image, verify, write_new};
use bincode::Options;
use risc0_zkvm::sha::Digestible;
use risc0_zkvm::{
    Executor, ExecutorEnv, ExternalProver, Prover, ProverOpts, Receipt, ReceiptClaim,
    VerifierContext,
};
use sha2::{Digest as _, Sha256};
use std::{fs, path::Path, time::Instant};
use zutti_protocol::encoding::Canonical;
use zutti_wallet::spv_pool::SpvPoolWitness;

pub enum Dependencies<'a> {
    Genuine(&'a [Receipt]),
    Unresolved,
    Missing(usize),
    WrongImage,
    WrongJournal,
}
pub fn verify_dependencies(witness: &SpvPoolWitness, receipts: &[Receipt]) -> Result<()> {
    let required = witness.assumptions()?;
    if required.len() != receipts.len() {
        return Err("incomplete SPV pool evidence".into());
    }
    for (index, ((image, journal), receipt)) in required.iter().zip(receipts).enumerate() {
        let private = index == 0
            || matches!(
                witness.action,
                zutti_wallet::spv_pool::SpvAction::Settle { .. }
            );
        if private
            && !matches!(
                receipt.inner,
                risc0_zkvm::InnerReceipt::Succinct(_) | risc0_zkvm::InnerReceipt::Groth16(_)
            )
        {
            return Err("private SPV dependencies require zero-knowledge receipts".into());
        }
        verify(receipt, *image, journal)?;
    }
    Ok(())
}
pub fn run(
    program: &[u8],
    witness: &SpvPoolWitness,
    dependencies: Dependencies<'_>,
    out: &Path,
    prove: bool,
) -> Result<()> {
    let claim = witness.evaluate()?;
    let required = witness.assumptions()?;
    let negative = matches!(
        dependencies,
        Dependencies::Missing(_) | Dependencies::WrongImage | Dependencies::WrongJournal
    );
    let genuine = matches!(dependencies, Dependencies::Genuine(_));
    let failure_claim = if negative {
        let index = match dependencies {
            Dependencies::Missing(index) => index,
            Dependencies::WrongJournal => 1,
            _ => 0,
        };
        let (image, journal) = required.get(index).ok_or("missing negative dependency")?;
        Some(
            ReceiptClaim::ok(image.0, journal.clone())
                .digest()
                .to_string(),
        )
    } else {
        None
    };
    if prove && !genuine {
        return Err("proving requires genuine verified dependency receipts".into());
    }
    let input = witness.to_bytes()?;
    let mut builder = ExecutorEnv::builder();
    builder
        .write_slice(&(input.len() as u32).to_le_bytes())
        .write_slice(&input)
        .session_limit(Some(DEFAULT_CYCLES))
        .segment_limit_po2(20);
    match dependencies {
        Dependencies::Genuine(receipts) => {
            verify_dependencies(witness, receipts)?;
            for receipt in receipts {
                builder.add_assumption(receipt.clone());
            }
        }
        mode => {
            for (index, (mut image, mut journal)) in required.into_iter().enumerate() {
                if matches!(mode, Dependencies::Missing(missing) if missing == index) {
                    continue;
                }
                if index == 0 && matches!(mode, Dependencies::WrongImage) {
                    image.0[0] ^= 1;
                }
                if index == 1 && matches!(mode, Dependencies::WrongJournal) {
                    journal[0] ^= 1;
                }
                builder.add_assumption(ReceiptClaim::ok(image.0, journal));
            }
        }
    }
    fs::create_dir(out)?;
    write_new(&out.join("witness.bin"), &input)?;
    let journal = claim.to_bytes()?;
    write_new(&out.join("journal.bin"), &journal)?;
    let started = Instant::now();
    let prover = ExternalProver::new("zutti-spv-pool", "r0vm");
    let image = image(program)?;
    let mut cycles = None;
    if prove {
        let result = prover.prove_with_ctx(
            builder.build()?,
            &VerifierContext::default().with_dev_mode(false),
            program,
            &ProverOpts::succinct().with_dev_mode(false),
        )?;
        verify(&result.receipt, image, &journal)?;
        if !matches!(result.receipt.inner, risc0_zkvm::InnerReceipt::Succinct(_)) {
            return Err("unexpected SPV pool receipt format".into());
        }
        write_new(
            &out.join("receipt.bin"),
            &bincode::DefaultOptions::new()
                .with_fixint_encoding()
                .with_limit(crate::MAX_RECEIPT as u64)
                .serialize(&result.receipt)?,
        )?;
        cycles = Some(result.stats.user_cycles);
    } else if negative {
        let error = prover
            .execute(builder.build()?, program)
            .err()
            .ok_or("guest accepted invalid proof dependencies")?;
        if !error.to_string().contains(&format!(
            "sys_verify_integrity: no receipt found to resolve assumption: claim digest {}",
            failure_claim.ok_or("missing expected dependency digest")?
        )) {
            return Err(format!("guest failed outside dependency verification: {error}").into());
        }
    } else {
        let result = prover.execute(builder.build()?, program)?;
        if result.exit_code != risc0_zkvm::ExitCode::Halted(0) || result.journal.bytes != journal {
            return Err("SPV pool journal mismatch".into());
        }
        cycles = Some(result.cycles());
    }
    let result = serde_json::json!({"schema":1,"image_id":image.to_string(),"program_sha256":hex::encode(Sha256::digest(program)),"journal_sha256":hex::encode(Sha256::digest(&journal)),"witness_sha256":hex::encode(Sha256::digest(&input)),"guest_rejected":negative,"journal_matches":!negative,"cycles":cycles,"seconds":started.elapsed().as_secs_f64(),"proof_generated":prove,"proof_verified":prove,"unresolved_execution_assumptions":!genuine,"source_anchor_authenticated":false,"security_review_complete":false,"ethereum_wrapped":false,"settlement_authority":false,"paid_jobs":0});
    write_new(
        &out.join("result.json"),
        &serde_json::to_vec_pretty(&result)?,
    )?;
    println!("{result}");
    Ok(())
}
