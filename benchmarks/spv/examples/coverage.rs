use clap::Parser;
use risc0_zkvm::{Executor, ExecutorEnv, ExternalProver, ReceiptClaim, sha::Digestible};
use std::{fs, path::PathBuf};
use zcash_protocol::consensus::NetworkType;
use zutti_protocol::{Digest, Secret32, encoding::Canonical};
use zutti_source::spv::{
    CoveragePrevious, CoverageWitness, Evaluation, HEADER_BYTES, HeaderClaim, Policy, WindowTerms,
    Witness, parse_block,
};
use zutti_spv_prototype::{
    DEFAULT_CYCLES, Result, RunOptions, bounded, image, read_receipt, reject, run, write_new,
};
#[derive(Parser)]
struct Args {
    #[arg(long)]
    program: PathBuf,
    #[arg(long)]
    out: PathBuf,
    #[arg(long)]
    prove: bool,
    #[arg(long)]
    previous: Option<PathBuf>,
}
fn block(height: u32) -> Vec<u8> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let file = if height == 0 {
        root.join("tests/fixtures/block-test-0.txt")
    } else {
        root.join(format!("tests/fixtures/spv/testnet-{height}.hex"))
    };
    hex::decode(
        fs::read_to_string(file)
            .expect("public block fixture")
            .trim(),
    )
    .expect("block hex")
}
fn digest(byte: u8) -> Digest {
    Digest([byte; 32])
}
fn secret() -> Secret32 {
    Secret32::from_bytes([7; 32]).expect("public randomness")
}
fn target(height: u32) -> Digest {
    parse_block(&block(height), NetworkType::Test)
        .expect("public block")
        .txids()[0]
}
fn dependency_rejected(
    program: &[u8],
    witness: &Witness,
    available: Option<Vec<u8>>,
    out: &std::path::Path,
) -> Result<()> {
    fs::create_dir(out)?;
    let input = witness.to_bytes()?;
    write_new(&out.join("witness.bin"), &input)?;
    let image = image(program)?;
    let required = witness
        .required_journal()?
        .ok_or("missing dependency case")?;
    let expected = ReceiptClaim::ok(image.0, required).digest().to_string();
    let mut builder = ExecutorEnv::builder();
    builder
        .write_slice(&(input.len() as u32).to_le_bytes())
        .write_slice(&input)
        .session_limit(Some(DEFAULT_CYCLES))
        .segment_limit_po2(20);
    if let Some(journal) = available {
        builder.add_assumption(ReceiptClaim::ok(image.0, journal));
    }
    let error = ExternalProver::new("zutti-coverage-rejection", "r0vm")
        .execute(builder.build()?, program)
        .err()
        .ok_or("coverage guest accepted a changed dependency")?;
    if !error.to_string().contains(&format!(
        "sys_verify_integrity: no receipt found to resolve assumption: claim digest {expected}"
    )) {
        return Err(format!("unexpected coverage rejection: {error}").into());
    }
    write_new(
        &out.join("result.json"),
        &serde_json::to_vec_pretty(
            &serde_json::json!({"guest_rejected":true,"expected_dependency":expected,"proof_generated":false,"paid_jobs":0,"settlement_authority":false}),
        )?,
    )?;
    Ok(())
}
fn main() -> Result<()> {
    let args = Args::parse();
    if args.prove && args.previous.is_none() {
        return Err(
            "genuine coverage proving needs an exact header-one predecessor receipt".into(),
        );
    }
    fs::create_dir(&args.out)?;
    let program = bounded(&args.program, 32 * 1024 * 1024)?;
    let policy = Policy::new(image(&program)?, NetworkType::Test, 1, 3, 4)?;
    let previous = HeaderClaim::genesis(policy, &block(0)[..HEADER_BYTES])?
        .advance(&[block(1)[..HEADER_BYTES].to_vec()])?;
    let receipt = args
        .previous
        .as_ref()
        .map(|path| read_receipt(path))
        .transpose()?;
    let terms = WindowTerms::new(
        digest(2),
        digest(3),
        1,
        previous.tip()?,
        3,
        digest(4),
        digest(5),
    )?;
    for (name, txid) in [
        ("paid", target(2)),
        ("at-expiry", target(3)),
        ("absent", target(4)),
    ] {
        let first = Witness::Coverage(Box::new(CoverageWitness {
            terms: terms.clone(),
            txid,
            randomness: secret(),
            previous: CoveragePrevious::Prefix(previous.clone()),
            blocks: vec![block(2)],
            confirmations: vec![],
            finish: false,
        }));
        let Evaluation::Coverage(coverage) = first.evaluate()? else {
            return Err("expected coverage journal".into());
        };
        let proof = run(
            &program,
            &first,
            receipt.as_ref(),
            &args.out.join(format!("{name}-first")),
            RunOptions {
                prove: args.prove,
                cycle_limit: DEFAULT_CYCLES,
                public_header_receipt: false,
            },
        )?;
        let last = |coverage| {
            Witness::Coverage(Box::new(CoverageWitness {
                terms: terms.clone(),
                txid,
                randomness: secret(),
                previous: CoveragePrevious::Coverage(coverage),
                blocks: vec![block(3)],
                confirmations: vec![block(4)[..HEADER_BYTES].to_vec()],
                finish: true,
            }))
        };
        let complete = last(coverage.clone());
        run(
            &program,
            &complete,
            proof.as_ref(),
            &args.out.join(format!("{name}-complete")),
            RunOptions {
                prove: args.prove,
                cycle_limit: DEFAULT_CYCLES,
                public_header_receipt: false,
            },
        )?;
        if !args.prove && name == "paid" {
            dependency_rejected(
                &program,
                &complete,
                None,
                &args.out.join("missing-coverage-proof"),
            )?;
            let mut forged = coverage.clone();
            forged.payment = 0;
            dependency_rejected(
                &program,
                &last(forged),
                Some(coverage.to_bytes()?),
                &args.out.join("forgotten-payment-proof"),
            )?;
            let mut omitted = last(coverage.clone());
            let Witness::Coverage(witness) = &mut omitted else {
                unreachable!()
            };
            witness.blocks.clear();
            reject(
                &program,
                &omitted,
                None,
                &args.out.join("missing-last-block"),
            )?;
            let mut changed = last(coverage);
            let Witness::Coverage(witness) = &mut changed else {
                unreachable!()
            };
            witness.txid = target(4);
            reject(&program, &changed, None, &args.out.join("changed-target"))?;
        }
    }
    write_new(
        &args.out.join("scope.json"),
        &serde_json::to_vec_pretty(
            &serde_json::json!({"inputs":"Public historical Testnet blocks one through four and generated commitments","prepared_ironwood_payment":false,"source_anchor_authenticated":args.prove,"unresolved_execution_assumptions":!args.prove,"proof_generated":args.prove,"ethereum_wrapped":false,"end_to_end_swap":false,"settlement_authority":false,"paid_jobs":0}),
        )?,
    )?;
    Ok(())
}
