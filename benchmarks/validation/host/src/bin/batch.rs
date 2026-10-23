use clap::{Parser, ValueEnum};
use risc0_zkvm::{Executor, ExecutorEnv, ExternalProver};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf, time::Instant};
use zcash_protocol::consensus::TEST_NETWORK;
use zutti_protocol::encoding::{Reader, Writer};
use zutti_source::{authorization, block::ParsedBlock};
use zutti_validation_runner::{Result, bounded, program};

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Suite {
    OrchardOriginal,
    OrchardFixed,
    Ironwood,
}

#[derive(Debug, Parser)]
struct Args {
    #[arg(long, value_enum)]
    suite: Suite,
    #[arg(long)]
    out: PathBuf,
    #[arg(long)]
    program: Option<PathBuf>,
    #[arg(long, default_value_t = 2, value_parser = clap::value_parser!(u8).range(2..=16))]
    repetitions: u8,
    #[arg(long, default_value_t = 1 << 38, value_parser = clap::value_parser!(u64).range(1..=1 << 38))]
    cycle_limit: u64,
}

fn main() -> Result<()> {
    let args = Args::parse();
    if args.out.try_exists()? {
        return Err("profile output already exists".into());
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let fixtures = root.join("fixtures");
    let manifest: serde_json::Value =
        serde_json::from_slice(&bounded(&fixtures.join("manifest.json"), 65_536)?)?;
    let selections = match args.suite {
        Suite::OrchardOriginal => [(1886667, 0, 2), (1986500, 1, 6)],
        Suite::OrchardFixed => [(4119011, 0, 1), (4094265, 1, 2)],
        Suite::Ironwood => [(4469047, 0, 1), (4520552, 3, 2)],
    };
    let expected_version = match args.suite {
        Suite::OrchardOriginal => "InsecurePreNu6_2",
        Suite::OrchardFixed => "FixedPostNu6_2",
        Suite::Ironwood => "PostNu6_3",
    };
    let mut metadata = Vec::new();
    let mut writer = Writer::default();
    writer.fixed(b"ZUTTIVB1");
    writer.u8(selections.len() as u8);
    for (height, index, actions) in selections {
        let raw = bounded(&fixtures.join(format!("{height}.bin")), 2_000_000)?;
        let digest = hex::encode(Sha256::digest(&raw));
        let entry = manifest["blocks"]
            .as_array()
            .ok_or("invalid fixture manifest")?
            .iter()
            .find(|row| row["height"].as_u64() == Some(height))
            .ok_or("missing block fixture")?;
        if manifest["network"] != "testnet"
            || entry["sha256"] != digest
            || entry["bytes"].as_u64() != Some(raw.len() as u64)
        {
            return Err("public block fixture identity mismatch".into());
        }
        let block = ParsedBlock::parse(&raw, &TEST_NETWORK)?;
        let tx = block
            .transactions()
            .get(index)
            .ok_or("missing public transaction")?;
        let bundle = match args.suite {
            Suite::Ironwood => tx.ironwood_bundle(),
            _ => tx.orchard_bundle(),
        }
        .ok_or("fixture pool mismatch")?;
        if block.claimed_height() as u64 != height
            || bundle.actions().len() != actions
            || format!("{:?}", bundle.bundle_version().circuit_version()) != expected_version
            || tx
                .transparent_bundle()
                .is_some_and(|b| !b.is_coinbase() && !b.vin.is_empty())
        {
            return Err("fixture authorization workload mismatch".into());
        }
        authorization::verify(tx, &[])?;
        writer.u32(index.try_into()?);
        writer.u8(args.repetitions);
        writer.bytes(&raw)?;
        metadata.push(serde_json::json!({"height":height,"transaction_index":index,
            "actions":actions,"circuit_version":expected_version,"txid":tx.txid().to_string(),"block_sha256":digest}));
    }
    let input = writer.finish();
    let mut stdin = (input.len() as u32).to_le_bytes().to_vec();
    stdin.extend_from_slice(&input);
    let program = program("zutti-validation-batch", args.program.as_deref())?;
    fs::create_dir_all(&args.out)?;
    fs::write(args.out.join("program.elf"), &program)?;
    fs::write(args.out.join("stdin.bin"), &stdin)?;
    let mut builder = ExecutorEnv::builder();
    builder
        .write_slice(&stdin)
        .session_limit(Some(args.cycle_limit))
        .segment_limit_po2(20);
    let started = Instant::now();
    let session = ExternalProver::new("zutti-validation-batch", "r0vm")
        .execute(builder.build()?, &program)?;
    if session.exit_code != risc0_zkvm::ExitCode::Halted(0) {
        return Err("validation batch did not halt successfully".into());
    }
    let mut reader = Reader::new(&session.journal.bytes);
    if reader.fixed::<32>()? != Sha256::digest(&input).as_slice()
        || usize::from(reader.u8()?) != metadata.len()
    {
        return Err("validation batch journal identity mismatch".into());
    }
    let mut measured = 0_u64;
    for entry in &mut metadata {
        let parse_cycles = reader.u64()?;
        measured = measured
            .checked_add(parse_cycles)
            .ok_or("profile cycle overflow")?;
        if reader.u8()? != args.repetitions || parse_cycles == 0 {
            return Err("validation batch journal count mismatch".into());
        }
        let mut cycles = Vec::new();
        for _ in 0..args.repetitions {
            let count = reader.u64()?;
            measured = measured
                .checked_add(count)
                .ok_or("profile cycle overflow")?;
            if count == 0 {
                return Err("empty verification measurement".into());
            }
            cycles.push(count);
        }
        entry["parse_cycles"] = parse_cycles.into();
        entry["verification_cycles"] = serde_json::json!(cycles);
    }
    reader.finish()?;
    if measured > session.cycles() {
        return Err("profile phase cycles exceed the full execution".into());
    }
    let mut sources = serde_json::Map::new();
    let project = root.join("../..");
    for name in [
        "crates/source/src/authorization.rs",
        "crates/source/src/authorization/groth.rs",
        "crates/source/src/authorization/signatures.rs",
        "benchmarks/validation/src/batch.rs",
        "benchmarks/validation/Cargo.toml",
        "benchmarks/validation/Cargo.lock",
        "crates/prover/methods/source/vendor/halo2_proofs/provenance.json",
        "crates/prover/methods/source/vendor/maybe-rayon/provenance.json",
    ] {
        sources.insert(
            name.into(),
            hex::encode(Sha256::digest(fs::read(project.join(name))?)).into(),
        );
    }
    let report = serde_json::json!({"schema":1,"suite":format!("{:?}",args.suite),"network":"testnet",
        "runtime":"risc0-zkvm 3.0.5","cycles":session.cycles(),"segments":session.segments.len(),
        "padded_cycles":session.segments.iter().map(|s| 1_u64 << s.po2).sum::<u64>(),
        "seconds":started.elapsed().as_secs_f64(),"repetitions":args.repetitions,"cases":metadata,
        "source_sha256":sources,"image_id":risc0_binfmt::compute_image_id(&program)?.to_string(),
        "program_sha256":hex::encode(Sha256::digest(&program)),"stdin_sha256":hex::encode(Sha256::digest(&stdin)),
        "journal_sha256":hex::encode(Sha256::digest(&session.journal.bytes)),"journal_matches":true,
        "native_authorization_verified":true,"proof_generated":false,"source_state_admitted":false,"paid_jobs":0});
    fs::write(
        args.out.join("execution.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{report}");
    Ok(())
}
