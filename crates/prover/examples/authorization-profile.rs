use clap::Parser;
use risc0_zkvm::{Executor, ExecutorEnv, ExternalProver};
use sha2::{Digest, Sha256};
use std::{fs, io::Read, path::PathBuf, time::Instant};
use zutti_protocol::encoding::Writer;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Debug, Parser)]
struct Args {
    #[arg(long)]
    fixture: PathBuf,
    #[arg(long)]
    out: PathBuf,
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u16).range(1..=64))]
    count: u16,
    #[arg(long)]
    accelerated: bool,
    #[arg(long, requires = "accelerated")]
    windowed: bool,
    #[arg(long)]
    profile: bool,
}

fn bounded(path: &std::path::Path, limit: usize) -> Result<Vec<u8>> {
    let file = fs::File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err("profile input must be a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err("profile input exceeds its bound".into());
    }
    Ok(bytes)
}

fn main() -> Result<()> {
    let args = Args::parse();
    if args.out.try_exists()? {
        return Err("profile output already exists".into());
    }
    let fixture_bytes = bounded(&args.fixture, 65_536)?;
    let fixture: serde_json::Value = serde_json::from_slice(&fixture_bytes)?;
    if fixture["public_data"] != true {
        return Err("the authorization profiler requires public fixtures".into());
    }
    let (kind, arity) = match fixture["kind"].as_str() {
        Some("sapling-output") => (0, 5),
        Some("sapling-spend") => (1, 7),
        Some("sprout") => (2, 9),
        _ => return Err("unsupported profile fixture kind".into()),
    };
    let proofs = fixture["proofs"]
        .as_array()
        .ok_or("missing profile proofs")?;
    if proofs.is_empty() || proofs.len() > 64 {
        return Err("invalid profile fixture count".into());
    }
    let mut writer = Writer::default();
    writer.fixed(b"ZUTTIG16");
    writer.u8(kind);
    writer.u16(args.count);
    for index in 0..usize::from(args.count) {
        let item = &proofs[index % proofs.len()];
        let proof = hex::decode(item["proof"].as_str().ok_or("missing proof bytes")?)?;
        if proof.len() != 192 {
            return Err("invalid profile proof length".into());
        }
        writer.fixed(&proof);
        let inputs = item["inputs"].as_array().ok_or("missing proof inputs")?;
        if inputs.len() != arity {
            return Err("invalid profile input count".into());
        }
        for input in inputs {
            let bytes = hex::decode(input.as_str().ok_or("invalid profile input")?)?;
            if bytes.len() != 32 {
                return Err("invalid profile scalar length".into());
            }
            writer.fixed(&bytes);
        }
    }
    let input = writer.finish();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target = risc0_build::get_target_dir(root.join("Cargo.toml"))
        .join("authorization-profile")
        .join(if args.windowed {
            "windowed"
        } else if args.accelerated {
            "accelerated"
        } else {
            "reference"
        });
    let mut build = risc0_build::cargo_command("build", &[]);
    build
        .arg("--locked")
        .arg("--manifest-path")
        .arg(root.join("benchmarks/authorization/Cargo.toml"))
        .arg("--target-dir")
        .arg(&target)
        .arg("--release");
    if args.windowed {
        build.args(["--features", "windowed-inputs"]);
    } else if args.accelerated {
        build.args(["--features", "accelerated-proofs"]);
    }
    if !build.status()?.success() {
        return Err("authorization profile guest compilation failed".into());
    }
    let user = bounded(
        &target.join("riscv32im-risc0-zkvm-elf/release/zutti-authorization-profile"),
        32 * 1024 * 1024,
    )?;
    let kernel = risc0_build::GuestOptions::default().kernel();
    let program = risc0_binfmt::ProgramBinary::new(&user, &kernel).encode();
    let mut stdin = (input.len() as u32).to_le_bytes().to_vec();
    stdin.extend_from_slice(&input);
    fs::create_dir_all(&args.out)?;
    fs::write(args.out.join("program.elf"), &program)?;
    fs::write(args.out.join("stdin.bin"), &stdin)?;
    let expected = Sha256::digest(&input);
    let mut builder = ExecutorEnv::builder();
    builder
        .write_slice(&stdin)
        .session_limit(Some(1 << 31))
        .segment_limit_po2(20);
    if args.profile {
        builder.enable_profiler(args.out.join("profile.pb"));
    }
    let started = Instant::now();
    let session = ExternalProver::new("zutti-authorization-profile", "r0vm")
        .execute(builder.build()?, &program)?;
    if session.exit_code != risc0_zkvm::ExitCode::Halted(0)
        || session.journal.bytes != expected.as_slice()
    {
        return Err("authorization profile execution failed or journal mismatched".into());
    }
    fs::write(args.out.join("journal.bin"), &session.journal.bytes)?;
    let report = serde_json::json!({"kind":fixture["kind"],"proof_count":args.count,
        "distinct_proofs":usize::from(args.count).min(proofs.len()),"accelerated":args.accelerated,
        "windowed_inputs":args.windowed,
        "cycles":session.cycles(),"segments":session.segments.len(),
        "padded_cycles":session.segments.iter().map(|s|1_u64 << s.po2).sum::<u64>(),
        "seconds":started.elapsed().as_secs_f64(),
        "image_id":risc0_binfmt::compute_image_id(&program)?.to_string(),
        "fixture_sha256":hex::encode(Sha256::digest(&fixture_bytes)),
        "stdin_sha256":hex::encode(Sha256::digest(&stdin)),"journal_matches":true,
        "proof_generated":false,"source_state_admitted":false,"settlement_authority":false});
    fs::write(
        args.out.join("execution.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", report);
    Ok(())
}
