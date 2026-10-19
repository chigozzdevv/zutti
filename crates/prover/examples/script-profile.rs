use clap::Parser;
use risc0_zkvm::{Executor, ExecutorEnv, ExternalProver};
use sha2::{Digest, Sha256};
use std::{fs, io::Read, path::PathBuf, time::Instant};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Debug, Parser)]
struct Args {
    #[arg(long)]
    out: PathBuf,
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u16).range(1..=64))]
    count: u16,
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
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target = risc0_build::get_target_dir(root.join("Cargo.toml"))
        .join("script-profile")
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
        .arg(root.join("benchmarks/scripts/Cargo.toml"))
        .arg("--target-dir")
        .arg(&target)
        .arg("--release");
    if args.accelerated {
        build.args(["--features", "accelerated-scripts"]);
    }
    if !build.status()?.success() {
        return Err("script profile guest compilation failed".into());
    }
    let file =
        fs::File::open(target.join("riscv32im-risc0-zkvm-elf/release/zutti-script-profile"))?;
    let mut user = Vec::new();
    file.take(32 * 1024 * 1024 + 1).read_to_end(&mut user)?;
    if user.is_empty() || user.len() > 32 * 1024 * 1024 {
        return Err("script profile guest size is invalid".into());
    }
    let program =
        risc0_binfmt::ProgramBinary::new(&user, &risc0_build::GuestOptions::default().kernel())
            .encode();
    let mut writer = zutti_protocol::encoding::Writer::default();
    writer.fixed(b"ZUTTISIG");
    writer.u16(args.count);
    let mut fixtures = Vec::new();
    for index in 1..=args.count {
        let seed = index as u8;
        let secret = secp256k1::SecretKey::from_secret_bytes([seed; 32])?;
        let digest = match index % 3 {
            0 => [0; 32],
            1 => [255; 32],
            _ => [seed; 32],
        };
        let mut public = match index % 3 {
            0 => secret.public_key().serialize().to_vec(),
            _ => secret.public_key().serialize_uncompressed().to_vec(),
        };
        if index % 3 == 2 {
            public[0] = 6 | (public[64] & 1);
        }
        let mut signature =
            secp256k1::ecdsa::sign(secp256k1::Message::from_digest(digest), &secret);
        if index % 2 == 0 {
            let mut compact = signature.serialize_compact();
            let scalar = secp256k1::SecretKey::from_secret_bytes(compact[32..].try_into()?)?
                .negate()
                .to_secret_bytes();
            compact[32..].copy_from_slice(&scalar);
            signature = secp256k1::ecdsa::Signature::from_compact(&compact)?;
        }
        let mut encoded = signature.serialize_der().to_vec();
        encoded.push(1);
        let decoded = match zcash_script::signature::Decoded::from_bytes(&encoded, false, false) {
            zcash_script::signature::Validity::Valid(value) => value,
            _ => return Err("invalid generated signature".into()),
        };
        let hash =
            |_: &zcash_script::script::Code, _: &zcash_script::signature::HashType| Some(digest);
        let verifier = zcash_script::interpreter::CallbackTransactionSignatureChecker {
            sighash: &hash,
            lock_time: 123,
            is_final: false,
        };
        if !zcash_script::interpreter::SignatureChecker::check_sig(
            &verifier,
            &decoded,
            &public,
            &zcash_script::script::Code(vec![]),
        ) {
            return Err("generated signature did not verify natively".into());
        }
        writer.fixed(&digest);
        writer.bytes(&public)?;
        writer.bytes(&encoded)?;
        fixtures.push(serde_json::json!({"digest":hex::encode(digest),
            "public_key":hex::encode(public),"signature":hex::encode(encoded),
            "high_s":index%2==0}));
    }
    let input = writer.finish();
    let expected = Sha256::digest(&input);
    let mut stdin = (input.len() as u32).to_le_bytes().to_vec();
    stdin.extend_from_slice(&input);
    fs::create_dir_all(&args.out)?;
    fs::write(args.out.join("program.elf"), &program)?;
    fs::write(args.out.join("stdin.bin"), &stdin)?;
    fs::write(
        args.out.join("fixtures.json"),
        serde_json::to_vec_pretty(&fixtures)?,
    )?;
    let mut builder = ExecutorEnv::builder();
    builder
        .write_slice(&stdin)
        .session_limit(Some(1 << 31))
        .segment_limit_po2(20);
    let started = Instant::now();
    let session =
        ExternalProver::new("zutti-script-profile", "r0vm").execute(builder.build()?, &program)?;
    if session.exit_code != risc0_zkvm::ExitCode::Halted(0)
        || session.journal.bytes != expected.as_slice()
    {
        return Err("script profile execution failed or journal mismatched".into());
    }
    fs::write(args.out.join("journal.bin"), &session.journal.bytes)?;
    let report = serde_json::json!({
        "signature_count":args.count,"accelerated":args.accelerated,
        "public_data":true,"generated_inputs":true,
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
    if args.check_invalid {
        let fixture = &fixtures[0];
        let digest = hex::decode(fixture["digest"].as_str().ok_or("missing digest")?)?;
        let public = hex::decode(fixture["public_key"].as_str().ok_or("missing public key")?)?;
        let signature = hex::decode(fixture["signature"].as_str().ok_or("missing signature")?)?;
        check_invalid(&args.out, &program, &digest, &public, &signature)?;
    }
    println!("{report}");
    Ok(())
}

fn check_invalid(
    out: &std::path::Path,
    program: &[u8],
    digest: &[u8],
    public: &[u8],
    signature: &[u8],
) -> Result<()> {
    let mut changed = digest.to_vec();
    changed[0] ^= 1;
    let mut malformed = public.to_vec();
    malformed[0] = 0;
    let mut bad_der = signature.to_vec();
    bad_der[0] = 0;
    let mut zero = secp256k1::ecdsa::Signature::from_compact(&[0; 64])?
        .serialize_der()
        .to_vec();
    zero.push(1);
    let cases = [
        (
            "changed_digest",
            changed,
            public.to_vec(),
            signature.to_vec(),
        ),
        (
            "malformed_key",
            digest.to_vec(),
            malformed,
            signature.to_vec(),
        ),
        ("malformed_der", digest.to_vec(), public.to_vec(), bad_der),
        ("zero_scalars", digest.to_vec(), public.to_vec(), zero),
    ];
    let mut reports = Vec::new();
    for (name, digest, public, signature) in cases {
        let mut writer = zutti_protocol::encoding::Writer::default();
        writer.fixed(b"ZUTTISIG");
        writer.u16(1);
        writer.fixed(&digest);
        writer.bytes(&public)?;
        writer.bytes(&signature)?;
        let body = writer.finish();
        let mut stdin = (body.len() as u32).to_le_bytes().to_vec();
        stdin.extend_from_slice(&body);
        fs::write(out.join(format!("invalid-{name}.bin")), &stdin)?;
        let mut builder = ExecutorEnv::builder();
        builder
            .write_slice(&stdin)
            .session_limit(Some(1 << 31))
            .segment_limit_po2(20);
        let error = ExternalProver::new("zutti-script-profile", "r0vm")
            .execute(builder.build()?, program)
            .err()
            .ok_or("invalid signature input was accepted by the guest")?;
        let error = format!("{error:#}");
        if !error.contains("Guest panicked") || !error.contains("invalid signature") {
            return Err(format!("unexpected invalid-input failure for {name}: {error}").into());
        }
        reports.push(serde_json::json!({"case":name,"guest_rejected":true,
            "stdin_sha256":hex::encode(Sha256::digest(&stdin)),"error":error}));
    }
    fs::write(
        out.join("rejections.json"),
        serde_json::to_vec_pretty(&reports)?,
    )?;
    Ok(())
}
