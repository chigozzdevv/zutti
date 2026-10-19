use clap::Parser;
use risc0_zkvm::{Executor, ExecutorEnv, ExternalProver};
use sha2::{Digest, Sha256};
use std::{fs, io::Read, path::PathBuf, time::Instant};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Debug, Parser)]
struct Args {
    #[arg(long)]
    fixture: PathBuf,
    #[arg(long)]
    out: PathBuf,
    #[arg(long, default_value_t = 4, value_parser = clap::value_parser!(u8).range(1..=64))]
    count: u8,
    #[arg(long)]
    accelerated: bool,
    #[arg(long)]
    check_invalid: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    if args.out.try_exists()? {
        return Err("profile output already exists".into());
    }
    let mut fixture = Vec::new();
    fs::File::open(&args.fixture)?
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut fixture)?;
    if fixture.len() > 2 * 1024 * 1024 {
        return Err("fixture is too large".into());
    }
    let fixtures: Vec<serde_json::Value> = serde_json::from_slice(&fixture)?;
    let selected = fixtures
        .get(..usize::from(args.count))
        .ok_or("not enough distinct fixture signatures")?;
    let mut stdin = vec![args.count];
    let mut seen = std::collections::BTreeSet::new();
    for fixture in selected {
        let field = |name: &str| -> Result<Vec<u8>> {
            Ok(hex::decode(
                fixture[name].as_str().ok_or("missing signature field")?,
            )?)
        };
        let key: [u8; 32] = field("key")?.try_into().map_err(|_| "invalid key length")?;
        let message = field("digest")?;
        let signature: [u8; 64] = field("signature")?
            .try_into()
            .map_err(|_| "invalid signature length")?;
        if message.len() != 32 {
            return Err("invalid digest length".into());
        }
        redjubjub::VerificationKey::<redjubjub::Binding>::try_from(key)?
            .verify(&message, &redjubjub::Signature::from(signature))?;
        let mut record = key.to_vec();
        record.extend_from_slice(&message);
        record.extend_from_slice(&signature);
        if !seen.insert(record) {
            return Err("duplicate signature fixture".into());
        }
        stdin.extend_from_slice(&key);
        stdin.extend_from_slice(&message);
        stdin.extend_from_slice(&signature);
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target = risc0_build::get_target_dir(root.join("Cargo.toml"))
        .join("sapling-signature-profile")
        .join(if args.accelerated {
            "accelerated"
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
        .arg(root.join("benchmarks/sapling-signatures/Cargo.toml"))
        .arg("--target-dir")
        .arg(&target)
        .arg("--release");
    if args.accelerated {
        build.args(["--features", "accelerated"]);
    }
    if !build.status()?.success() {
        return Err("Sapling signature guest compilation failed".into());
    }
    let mut user = Vec::new();
    fs::File::open(
        target.join("riscv32im-risc0-zkvm-elf/release/zutti-sapling-signature-profile"),
    )?
    .take(32 * 1024 * 1024 + 1)
    .read_to_end(&mut user)?;
    if user.is_empty() || user.len() > 32 * 1024 * 1024 {
        return Err("profile guest size is invalid".into());
    }
    let program =
        risc0_binfmt::ProgramBinary::new(&user, &risc0_build::GuestOptions::default().kernel())
            .encode();
    fs::create_dir(&args.out)?;
    fs::write(args.out.join("program.elf"), &program)?;
    fs::write(args.out.join("stdin.bin"), &stdin)?;
    let started = Instant::now();
    let session = execute(&stdin, &program)?;
    let seconds = started.elapsed().as_secs_f64();
    if session.exit_code != risc0_zkvm::ExitCode::Halted(0)
        || session.journal.bytes != Sha256::digest(&stdin).as_slice()
    {
        return Err("profile execution failed or journal mismatched".into());
    }
    fs::write(args.out.join("journal.bin"), &session.journal.bytes)?;
    let report = serde_json::json!({"count":args.count,"accelerated":args.accelerated,
        "public_data":true,"distinct_signatures":true,"cycles":session.cycles(),
        "segments":session.segments.len(),"seconds":seconds,
        "padded_cycles":session.segments.iter().map(|s|1_u64<<s.po2).sum::<u64>(),
        "image_id":risc0_binfmt::compute_image_id(&program)?.to_string(),
        "program_sha256":hex::encode(Sha256::digest(&program)),
        "stdin_sha256":hex::encode(Sha256::digest(&stdin)),
        "journal_sha256":hex::encode(Sha256::digest(&session.journal.bytes)),
        "journal_matches":true,"proof_generated":false,"source_state_admitted":false,
        "settlement_authority":false});
    fs::write(
        args.out.join("execution.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    if args.check_invalid {
        let mut reports = Vec::new();
        for (name, range, fill) in [
            ("changed_message", 33..34, false),
            ("noncanonical_key", 1..33, true),
            ("noncanonical_r", 65..97, true),
            ("noncanonical_scalar", 97..129, true),
        ] {
            let mut changed = stdin[..129].to_vec();
            changed[0] = 1;
            if fill {
                changed[range].fill(255);
            } else {
                changed[range.start] ^= 1;
            }
            fs::write(args.out.join(format!("invalid-{name}.bin")), &changed)?;
            let error = execute(&changed, &program)
                .err()
                .ok_or("invalid signature input was accepted")?;
            let error = format!("{error:#}");
            if !error.contains("Guest panicked: invalid signature") {
                return Err(format!("unexpected failure for {name}: {error}").into());
            }
            reports.push(serde_json::json!({"case":name,"guest_rejected":true,
                "stdin_sha256":hex::encode(Sha256::digest(&changed)),"error":error}));
        }
        fs::write(
            args.out.join("rejections.json"),
            serde_json::to_vec_pretty(&reports)?,
        )?;
    }
    println!("{report}");
    Ok(())
}

fn execute(stdin: &[u8], program: &[u8]) -> Result<risc0_zkvm::SessionInfo> {
    let mut builder = ExecutorEnv::builder();
    builder
        .write_slice(stdin)
        .session_limit(Some(1 << 31))
        .segment_limit_po2(20);
    Ok(
        ExternalProver::new("zutti-sapling-signature-profile", "r0vm")
            .execute(builder.build()?, program)?,
    )
}
