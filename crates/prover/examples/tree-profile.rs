use clap::Parser;
use risc0_zkvm::{Executor, ExecutorEnv, ExternalProver};
use sha2::{Digest, Sha256};
use std::{fs, io::Read, path::PathBuf, time::Instant};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Debug, Parser)]
struct Args {
    #[arg(long)]
    out: PathBuf,
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u16).range(1..=1024))]
    count: u16,
    #[arg(long)]
    windowed: bool,
    #[arg(long, requires = "windowed")]
    accelerated: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    if args.out.try_exists()? {
        return Err("profile output already exists".into());
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target = risc0_build::get_target_dir(root.join("Cargo.toml"))
        .join("tree-profile")
        .join(if args.accelerated {
            "accelerated"
        } else if args.windowed {
            "windowed"
        } else {
            "reference"
        });
    let mut build = risc0_build::cargo_command("build", &[]);
    if let Some(home) = std::env::var_os("CARGO_HOME") {
        build.env("CARGO_HOME", home);
    }
    build
        .arg("--locked")
        .arg("--manifest-path")
        .arg(root.join("benchmarks/trees/Cargo.toml"))
        .arg("--target-dir")
        .arg(&target)
        .arg("--release");
    if args.accelerated {
        build.args(["--features", "accelerated-trees"]);
    } else if args.windowed {
        build.args(["--features", "windowed-trees"]);
    }
    if !build.status()?.success() {
        return Err("tree profile guest compilation failed".into());
    }
    let file = fs::File::open(target.join("riscv32im-risc0-zkvm-elf/release/zutti-tree-profile"))?;
    let mut user = Vec::new();
    file.take(32 * 1024 * 1024 + 1).read_to_end(&mut user)?;
    if user.is_empty() || user.len() > 32 * 1024 * 1024 {
        return Err("tree profile guest size is invalid".into());
    }
    let program =
        risc0_binfmt::ProgramBinary::new(&user, &risc0_build::GuestOptions::default().kernel())
            .encode();
    let mut stdin = args.count.to_le_bytes().to_vec();
    stdin.extend_from_slice(&[42; 32]);
    stdin.extend_from_slice(&[91; 32]);
    let mut expected = [42; 32];
    for index in 0..usize::from(args.count) {
        expected = sapling::merkle_hash(index % 32, &expected, &[91; 32]);
    }
    fs::create_dir_all(&args.out)?;
    fs::write(args.out.join("program.elf"), &program)?;
    fs::write(args.out.join("stdin.bin"), &stdin)?;
    let mut builder = ExecutorEnv::builder();
    builder
        .write_slice(&stdin)
        .session_limit(Some(1 << 31))
        .segment_limit_po2(20);
    let started = Instant::now();
    let session =
        ExternalProver::new("zutti-tree-profile", "r0vm").execute(builder.build()?, &program)?;
    if session.exit_code != risc0_zkvm::ExitCode::Halted(0) || session.journal.bytes != expected {
        return Err("tree profile execution failed or journal mismatched".into());
    }
    fs::write(args.out.join("journal.bin"), &session.journal.bytes)?;
    let report = serde_json::json!({
        "hash_count":args.count,"windowed_trees":args.windowed,
        "accelerated_trees":args.accelerated,"public_data":true,
        "cycles":session.cycles(),"segments":session.segments.len(),
        "padded_cycles":session.segments.iter().map(|s|1_u64 << s.po2).sum::<u64>(),
        "seconds":started.elapsed().as_secs_f64(),
        "image_id":risc0_binfmt::compute_image_id(&program)?.to_string(),
        "program_sha256":hex::encode(Sha256::digest(&program)),
        "stdin_sha256":hex::encode(Sha256::digest(&stdin)),
        "journal_sha256":hex::encode(Sha256::digest(expected)),"journal_matches":true,
        "proof_generated":false,"source_state_admitted":false,"settlement_authority":false
    });
    fs::write(
        args.out.join("execution.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{report}");
    Ok(())
}
