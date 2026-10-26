use clap::{Parser, Subcommand, ValueEnum};
use sha2::{Digest as _, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
use zcash_protocol::consensus::{NetworkType, TEST_NETWORK};
use zutti_protocol::{Digest, Secret32, encoding::Canonical};
use zutti_source::{
    block::ParsedBlock,
    spv::{
        AdmissionWitness, Branch, HEADER_BYTES, HeaderClaim, Policy, WindowTerms, WindowWitness,
        Witness,
    },
};
use zutti_spv_prototype::{
    DEFAULT_CYCLES, MAX_CYCLES, Result, RunOptions, audit, bounded, image, read_receipt, reject,
    run, run_admission, verify, verify_header_receipt, write_new,
};

#[derive(Debug, Parser)]
#[command(name = "zutti-spv-prototype")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Checkpoint {
        #[arg(long)]
        headers: PathBuf,
        #[arg(long)]
        height: u32,
        #[arg(long)]
        work: String,
        #[arg(long)]
        out: PathBuf,
    },
    Boundless {
        #[command(subcommand)]
        command: BoundlessCommand,
    },
    PrepareAdmission {
        #[arg(long)]
        header_program: PathBuf,
        #[arg(long)]
        previous: PathBuf,
        #[arg(long)]
        confirmations: PathBuf,
        #[arg(long)]
        stable_headers: Option<PathBuf>,
        #[arg(long)]
        out: PathBuf,
    },
    RunAdmission {
        #[arg(long)]
        program: PathBuf,
        #[arg(long)]
        witness: PathBuf,
        #[arg(long)]
        previous: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        prove: bool,
    },
    Bootstrap {
        #[command(subcommand)]
        command: BootstrapCommand,
    },
    Build {
        #[arg(long, value_enum, default_value = "source")]
        kind: Guest,
        #[arg(long)]
        boundless: bool,
        #[arg(long)]
        guest_target: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    Admission {
        #[arg(long)]
        program: PathBuf,
        #[arg(long)]
        header_program: PathBuf,
        #[arg(long)]
        previous: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        prove: bool,
    },
    Demo {
        #[arg(long, default_value_t = 20, value_parser=clap::value_parser!(u32).range(16..=22))]
        segment_po2: u32,
        #[arg(long)]
        program: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        prove: bool,
        #[arg(long, requires = "prove")]
        public_header_receipt: bool,
        #[arg(long, value_enum, default_value = "all")]
        case: Case,
        #[arg(long)]
        previous: Option<PathBuf>,
        #[arg(long, default_value_t=DEFAULT_CYCLES, value_parser=clap::value_parser!(u64).range(1..=MAX_CYCLES))]
        cycle_limit: u64,
    },
    Run {
        #[arg(long, default_value_t = 20, value_parser=clap::value_parser!(u32).range(16..=22))]
        segment_po2: u32,
        #[arg(long)]
        program: PathBuf,
        #[arg(long)]
        witness: PathBuf,
        #[arg(long)]
        previous: Option<PathBuf>,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        prove: bool,
        #[arg(long, requires = "prove")]
        public_header_receipt: bool,
        #[arg(long, default_value_t=DEFAULT_CYCLES, value_parser=clap::value_parser!(u64).range(1..=MAX_CYCLES))]
        cycle_limit: u64,
    },
    Binding {
        #[arg(long)]
        program: PathBuf,
        #[arg(long)]
        witness: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        prove: bool,
    },
    Wrap {
        #[arg(long)]
        image: String,
        #[arg(long)]
        receipt: PathBuf,
        #[arg(long)]
        journal: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    Pool {
        #[arg(long)]
        program: PathBuf,
        #[arg(long)]
        witness: PathBuf,
        #[arg(long = "receipt")]
        receipts: Vec<PathBuf>,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        prove: bool,
    },
    Verify {
        #[arg(long)]
        program: PathBuf,
        #[arg(long)]
        receipt: PathBuf,
        #[arg(long)]
        journal: PathBuf,
    },
    Audit {
        #[arg(long)]
        program: PathBuf,
        #[arg(long)]
        receipt: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
enum BoundlessCommand {
    Export {
        #[arg(long, value_enum)]
        kind: zutti_spv_prototype::boundless::Kind,
        #[arg(long)]
        program: PathBuf,
        #[arg(long)]
        witness: PathBuf,
        #[arg(long)]
        dependency: Vec<PathBuf>,
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..=zutti_spv_prototype::boundless::MAX_CYCLES))]
        cycles: u64,
        #[arg(long, required = true)]
        public: bool,
        #[arg(long)]
        out: PathBuf,
    },
    Execute {
        #[arg(long)]
        job: PathBuf,
    },
    Import {
        #[arg(long)]
        job: PathBuf,
        #[arg(long)]
        proof: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
enum BootstrapCommand {
    Init {
        #[arg(long)]
        state: PathBuf,
        #[arg(long)]
        program: PathBuf,
        #[arg(
            long,
            required_unless_present = "checkpoint",
            conflicts_with = "checkpoint"
        )]
        genesis: Option<PathBuf>,
        #[arg(long, required_unless_present = "genesis", conflicts_with = "genesis")]
        checkpoint: Option<PathBuf>,
        #[arg(long)]
        confirmations: u32,
        #[arg(long)]
        window_blocks: u32,
        #[arg(long)]
        valid_through: u32,
    },
    Prepare {
        #[arg(long)]
        state: PathBuf,
        #[arg(long)]
        headers: Option<PathBuf>,
    },
    Accept {
        #[arg(long)]
        state: PathBuf,
        #[arg(long)]
        batch: u32,
        #[arg(long)]
        receipt: PathBuf,
    },
    Status {
        #[arg(long)]
        state: PathBuf,
    },
    Prove {
        #[arg(long, default_value_t = 20, value_parser=clap::value_parser!(u32).range(16..=22))]
        segment_po2: u32,
        #[arg(long)]
        state: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        public_header_receipt: bool,
        #[arg(long, default_value_t=DEFAULT_CYCLES, value_parser=clap::value_parser!(u64).range(1..=MAX_CYCLES))]
        cycle_limit: u64,
    },
}

fn bootstrap(command: BootstrapCommand) -> Result<()> {
    use zutti_spv_prototype::bootstrap::{Bootstrap, headers};
    let status = match command {
        BootstrapCommand::Init {
            state,
            program,
            genesis,
            checkpoint,
            confirmations,
            window_blocks,
            valid_through,
        } => {
            let program = bounded(&program, 32 * 1024 * 1024)?;
            let policy = Policy::new(
                image(&program)?,
                NetworkType::Test,
                confirmations,
                window_blocks,
                valid_through,
            )?;
            match (genesis, checkpoint) {
                (Some(genesis), None) => {
                    Bootstrap::init(&state, &program, &bounded(&genesis, HEADER_BYTES)?, policy)?
                        .status()?
                }
                (None, Some(path)) => {
                    let checkpoint = HeaderClaim::from_bytes(&bounded(&path, 16_384)?)?;
                    let policy = Policy::from_checkpoint(
                        image(&program)?,
                        confirmations,
                        window_blocks,
                        valid_through,
                        checkpoint.checkpoint_id()?,
                    )?;
                    Bootstrap::init_checkpoint(&state, &program, &checkpoint, policy)?.status()?
                }
                _ => return Err("choose exactly one bootstrap origin".into()),
            }
        }
        BootstrapCommand::Prepare {
            state,
            headers: input,
        } => {
            let state = Bootstrap::open(&state)?;
            let path = state.prepare(
                input
                    .as_deref()
                    .map(headers)
                    .transpose()?
                    .unwrap_or_default(),
            )?;
            println!("{}", serde_json::json!({"prepared_batch":path}));
            state.status()?
        }
        BootstrapCommand::Accept {
            state,
            batch,
            receipt,
        } => Bootstrap::open(&state)?.accept(batch, &receipt)?,
        BootstrapCommand::Status { state } => Bootstrap::open(&state)?.status()?,
        BootstrapCommand::Prove {
            state,
            out,
            cycle_limit,
            public_header_receipt,
            segment_po2,
        } => Bootstrap::open(&state)?.prove(
            &out,
            RunOptions {
                prove: true,
                cycle_limit,
                public_header_receipt,
                segment_po2,
            },
        )?,
    };
    println!("{}", serde_json::to_string(&status)?);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Case {
    All,
    Genesis,
    FirstHeader,
    Headers,
    Included,
    Absent,
    FalseAbsence,
    MissingBlock,
    WrongParent,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Guest {
    Source,
    Admission,
    Binding,
    Pool,
    ReceiptCheck,
}

fn root() -> Result<PathBuf> {
    Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()?)
}

fn fixture(height: u32) -> Result<Vec<u8>> {
    let path = if height == 0 {
        root()?.join("tests/fixtures/block-test-0.txt")
    } else {
        root()?.join(format!("tests/fixtures/spv/testnet-{height}.hex"))
    };
    Ok(hex::decode(
        std::str::from_utf8(&bounded(&path, 20_000)?)?.trim(),
    )?)
}

fn build(target: &Path, out: &Path, kind: Guest, boundless: bool) -> Result<()> {
    if out.try_exists()? {
        return Err("build output already exists".into());
    }
    let root = root()?;
    let cargo_home =
        PathBuf::from(std::env::var_os("CARGO_HOME").ok_or("set CARGO_HOME explicitly")?);
    let mut flags = vec![format!("--remap-path-prefix={}=/zutti", root.display())];
    for path in [cargo_home.clone(), cargo_home.canonicalize()?] {
        let flag = format!("--remap-path-prefix={}=/cargo", path.display());
        if !flags.contains(&flag) {
            flags.push(flag);
        }
    }
    let name = match kind {
        Guest::Source => "spv_source",
        Guest::Admission => "spv_admission",
        Guest::Binding => "payment_binding",
        Guest::Pool => "spv_pool",
        Guest::ReceiptCheck => "spv_receipt_check",
    };
    let manifest = match kind {
        Guest::Binding => "crates/prover/methods/binding/Cargo.toml",
        Guest::Pool => "crates/prover/methods/spv-pool/Cargo.toml",
        _ => "crates/prover/methods/spv-source/Cargo.toml",
    };
    let mut command = risc0_build::cargo_command("build", &flags);
    if boundless {
        if matches!(kind, Guest::Binding) {
            return Err("binding is stateless and uses its existing guest".into());
        }
        command.args(["--features", "boundless"]);
    }
    let status = command
        .env("CARGO_HOME", &cargo_home)
        .env("CARGO_NET_OFFLINE", "true")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .args([
            "--locked",
            "--offline",
            "--release",
            "--bin",
            name,
            "-j",
            "1",
            "--manifest-path",
        ])
        .arg(root.join(manifest))
        .arg("--target-dir")
        .arg(target)
        .status()?;
    if !status.success() {
        return Err("SPV guest build failed".into());
    }
    let user = bounded(
        &target.join("riscv32im-risc0-zkvm-elf/release").join(name),
        32 * 1024 * 1024,
    )?;
    let program =
        risc0_binfmt::ProgramBinary::new(&user, &risc0_build::GuestOptions::default().kernel())
            .encode();
    fs::create_dir(out)?;
    write_new(&out.join("program.elf"), &program)?;
    let manifest = serde_json::json!({"schema":1,"image_id":image(&program)?.to_string(),
        "program_sha256":hex::encode(Sha256::digest(&program)),"admitted":false,"kind":name,"network_scope":"Testnet prototype",
        "composition":if boundless || matches!(kind, Guest::ReceiptCheck) {"inline-groth16"} else {"native"}});
    write_new(
        &out.join("program.json"),
        &serde_json::to_vec_pretty(&manifest)?,
    )?;
    println!("{manifest}");
    Ok(())
}

fn demo(
    program: &Path,
    out: &Path,
    options: RunOptions,
    case: Case,
    previous: Option<&Path>,
) -> Result<()> {
    if options.public_header_receipt
        && !matches!(case, Case::Genesis | Case::FirstHeader | Case::Headers)
    {
        return Err("composite demo mode supports only public genesis or headers".into());
    }
    if out.try_exists()?
        || (matches!(case, Case::Genesis | Case::FirstHeader) && previous.is_some())
    {
        return Err("demo output exists or predecessor is unexpected".into());
    }
    let program = bounded(program, 32 * 1024 * 1024)?;
    let policy = Policy::new(image(&program)?, NetworkType::Test, 1, 3, 4)?;
    let prefix = HeaderClaim::genesis(policy, &fixture(0)?[..HEADER_BYTES])?;
    let mut receipt = previous.map(read_receipt).transpose()?;
    if let Some(receipt) = &receipt {
        let claim = verify_header_receipt(&program, receipt)?;
        if claim.to_bytes()? != prefix.to_bytes()? {
            return Err("demo requires its exact genesis predecessor".into());
        }
    }
    fs::create_dir(out)?;
    if matches!(case, Case::All | Case::Genesis) && receipt.is_none() {
        let witness = Witness::Genesis {
            policy,
            header: fixture(0)?[..HEADER_BYTES].to_vec(),
            headers: vec![],
        };
        receipt = run(&program, &witness, None, &out.join("genesis"), options)?;
    }
    if matches!(case, Case::All | Case::Headers) {
        let witness = Witness::Headers {
            previous: prefix.clone(),
            headers: (1..=4)
                .map(|h| fixture(h).map(|b| b[..HEADER_BYTES].to_vec()))
                .collect::<Result<_>>()?,
        };
        run(
            &program,
            &witness,
            receipt.as_ref(),
            &out.join("headers"),
            options,
        )?;
    }
    if case == Case::FirstHeader {
        let witness = Witness::Genesis {
            policy,
            header: fixture(0)?[..HEADER_BYTES].to_vec(),
            headers: vec![fixture(1)?[..HEADER_BYTES].to_vec()],
        };
        run(&program, &witness, None, &out.join("first-header"), options)?;
    }
    for (selected, name, branch) in [
        (Case::Included, "included", Branch::Included),
        (Case::Absent, "absent", Branch::Absent),
    ] {
        if case != Case::All && case != selected {
            continue;
        }
        let terms = WindowTerms::new(
            Digest([2; 32]),
            Digest([3; 32]),
            0,
            prefix.tip()?,
            2,
            Digest([4; 32]),
            Digest([5; 32]),
        )?;
        let txid = if branch == Branch::Included {
            ParsedBlock::parse(&fixture(2)?, &TEST_NETWORK)?.txids()[0]
        } else {
            Digest([77; 32])
        };
        let headers = (1..=4)
            .map(|h| fixture(h).map(|b| b[..HEADER_BYTES].to_vec()))
            .collect::<Result<_>>()?;
        let heights = if branch == Branch::Included {
            vec![2]
        } else {
            vec![1, 2]
        };
        let blocks = heights.into_iter().map(fixture).collect::<Result<_>>()?;
        let window = WindowWitness::new(
            terms,
            txid,
            Secret32::from_bytes([6; 32])?,
            branch,
            headers,
            blocks,
        )?;
        let witness = Witness::Window {
            previous: prefix.clone(),
            window: Box::new(window),
        };
        run(
            &program,
            &witness,
            receipt.as_ref(),
            &out.join(name),
            options,
        )?;
    }
    for (selected, name) in [
        (Case::FalseAbsence, "false-absence"),
        (Case::MissingBlock, "missing-block"),
        (Case::WrongParent, "wrong-parent"),
    ] {
        if case != Case::All && case != selected {
            continue;
        }
        let witness = if selected == Case::WrongParent {
            Witness::Headers {
                previous: prefix.clone(),
                headers: vec![fixture(2)?[..HEADER_BYTES].to_vec()],
            }
        } else {
            let terms = WindowTerms::new(
                Digest([2; 32]),
                Digest([3; 32]),
                0,
                prefix.tip()?,
                2,
                Digest([4; 32]),
                Digest([5; 32]),
            )?;
            let target = if selected == Case::FalseAbsence {
                ParsedBlock::parse(&fixture(2)?, &TEST_NETWORK)?.txids()[0]
            } else {
                Digest([77; 32])
            };
            let headers = (1..=4)
                .map(|h| fixture(h).map(|b| b[..HEADER_BYTES].to_vec()))
                .collect::<Result<_>>()?;
            let blocks = if selected == Case::MissingBlock {
                vec![fixture(1)?]
            } else {
                vec![fixture(1)?, fixture(2)?]
            };
            Witness::Window {
                previous: prefix.clone(),
                window: Box::new(WindowWitness::new(
                    terms,
                    target,
                    Secret32::from_bytes([6; 32])?,
                    Branch::Absent,
                    headers,
                    blocks,
                )?),
            }
        };
        reject(&program, &witness, receipt.as_ref(), &out.join(name))?;
    }
    write_new(
        &out.join("scope.json"),
        &serde_json::to_vec_pretty(&serde_json::json!({
            "inputs":"Generated public order terms and real early Testnet blocks",
            "inclusion_sample":"Historical coinbase transaction; not a prepared shielded swap payment",
            "full_bootstrap":false,"end_to_end_swap":false,"settlement_authority":false,"paid_jobs":0,
        }))?,
    )?;
    Ok(())
}

fn main() -> Result<()> {
    match Args::parse().command {
        Command::Boundless { command } => {
            use zutti_spv_prototype::boundless;
            match command {
                BoundlessCommand::Export {
                    kind,
                    program,
                    witness,
                    dependency,
                    cycles,
                    public: _,
                    out,
                } => boundless::export(&program, &witness, &dependency, kind, cycles, &out),
                BoundlessCommand::Execute { job } => boundless::execute(&job),
                BoundlessCommand::Import { job, proof, out } => {
                    boundless::import(&job, &proof, &out)
                }
            }
        }
        Command::PrepareAdmission {
            header_program,
            previous,
            confirmations,
            stable_headers,
            out,
        } => {
            let program = bounded(&header_program, 32 * 1024 * 1024)?;
            let receipt = read_receipt(&previous)?;
            let prefix = verify_header_receipt(&program, &receipt)?;
            let confirmations = zutti_spv_prototype::bootstrap::headers(&confirmations)?;
            let witness = if let Some(headers) = stable_headers {
                AdmissionWitness::Advance {
                    previous: prefix,
                    stable_headers: zutti_spv_prototype::bootstrap::headers(&headers)?,
                    confirmations,
                }
            } else {
                AdmissionWitness::Initialize {
                    stable: prefix,
                    confirmations,
                }
            };
            let claim = witness.evaluate()?;
            if out.try_exists()? {
                return Err("admission preparation already exists".into());
            }
            let parent = out
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let stage = tempfile::Builder::new()
                .prefix(".spv-admission-")
                .tempdir_in(parent)?;
            write_new(&stage.path().join("witness.bin"), &witness.to_bytes()?)?;
            write_new(&stage.path().join("journal.bin"), &claim.to_bytes()?)?;
            fs::File::open(stage.path())?.sync_all()?;
            fs::rename(stage.path(), &out)?;
            fs::File::open(parent)?.sync_all()?;
            println!(
                "{}",
                serde_json::json!({"prepared":true,"stable_height":claim.stable.height,
                "head_height":claim.head.height,"predecessor_proof_verified":true,"proof_generated":false,
                "deployment_admitted":false})
            );
            Ok(())
        }
        Command::RunAdmission {
            program,
            witness,
            previous,
            out,
            prove,
        } => {
            let witness =
                AdmissionWitness::from_bytes(&bounded(&witness, zutti_source::spv::MAX_WITNESS)?)?;
            run_admission(
                &bounded(&program, 32 * 1024 * 1024)?,
                &witness,
                &read_receipt(&previous)?,
                &out,
                prove,
            )
        }
        Command::Bootstrap { command } => bootstrap(command),
        Command::Build {
            guest_target,
            out,
            kind,
            boundless,
        } => build(&guest_target, &out, kind, boundless),
        Command::Admission {
            program,
            header_program,
            previous,
            out,
            prove,
        } => {
            let header_program = bounded(&header_program, 32 * 1024 * 1024)?;
            let previous = read_receipt(&previous)?;
            let prefix = verify_header_receipt(&header_program, &previous)?;
            let policy = Policy::new(image(&header_program)?, NetworkType::Test, 1, 3, 4)?;
            if prefix.to_bytes()?
                != HeaderClaim::genesis(policy, &fixture(0)?[..HEADER_BYTES])?.to_bytes()?
            {
                return Err("admission demo requires its exact genesis predecessor".into());
            }
            let witness = AdmissionWitness::Advance {
                previous: prefix,
                stable_headers: (1..=3)
                    .map(|h| fixture(h).map(|b| b[..HEADER_BYTES].to_vec()))
                    .collect::<Result<_>>()?,
                confirmations: vec![fixture(4)?[..HEADER_BYTES].to_vec()],
            };
            run_admission(
                &bounded(&program, 32 * 1024 * 1024)?,
                &witness,
                &previous,
                &out,
                prove,
            )
        }
        Command::Demo {
            segment_po2,
            program,
            out,
            prove,
            public_header_receipt,
            case,
            previous,
            cycle_limit,
        } => demo(
            &program,
            &out,
            RunOptions {
                segment_po2,
                prove,
                cycle_limit,
                public_header_receipt,
            },
            case,
            previous.as_deref(),
        ),
        Command::Run {
            segment_po2,
            program,
            witness,
            previous,
            out,
            prove,
            public_header_receipt,
            cycle_limit,
        } => {
            let program = bounded(&program, 32 * 1024 * 1024)?;
            let witness = Witness::from_bytes(&bounded(&witness, zutti_source::spv::MAX_WITNESS)?)?;
            let receipt = previous.as_deref().map(read_receipt).transpose()?;
            run(
                &program,
                &witness,
                receipt.as_ref(),
                &out,
                RunOptions {
                    segment_po2,
                    prove,
                    cycle_limit,
                    public_header_receipt,
                },
            )?;
            Ok(())
        }
        Command::Binding {
            program,
            witness,
            out,
            prove,
        } => {
            use zutti_wallet::binding::{MAX_BINDING_WITNESS, PaymentBindingWitness};
            let program = bounded(&program, 32 * 1024 * 1024)?;
            let witness =
                PaymentBindingWitness::from_bytes(&bounded(&witness, MAX_BINDING_WITNESS)?)?;
            zutti_spv_prototype::binding::run(&program, &witness, &out, prove, false)
        }
        Command::Wrap {
            image,
            receipt,
            journal,
            out,
        } => {
            use bincode::Options;
            let image = Digest::from_hex(&image)?;
            let receipt = read_receipt(&receipt)?;
            let journal = bounded(&journal, 16_384)?;
            if out.try_exists()? {
                return Err("wrapping output already exists".into());
            }
            let (wrapped, seal) = zutti_spv_prototype::ethereum::wrap(&receipt, image, &journal)?;
            fs::create_dir(&out)?;
            write_new(
                &out.join("receipt.bin"),
                &bincode::DefaultOptions::new()
                    .with_fixint_encoding()
                    .with_limit(zutti_spv_prototype::MAX_RECEIPT as u64)
                    .serialize(&wrapped)?,
            )?;
            write_new(&out.join("seal.bin"), &seal)?;
            write_new(&out.join("journal.bin"), &journal)?;
            let result = serde_json::json!({"image_id":image.to_string(),"proof_verified":true,"format":"groth16","ethereum_seal_bytes":seal.len(),"transaction_submitted":false,"settlement_authority":false,"paid_jobs":0});
            write_new(
                &out.join("result.json"),
                &serde_json::to_vec_pretty(&result)?,
            )?;
            println!("{result}");
            Ok(())
        }
        Command::Pool {
            program,
            witness,
            receipts,
            out,
            prove,
        } => {
            use zutti_spv_prototype::settlement::{Dependencies, run};
            use zutti_wallet::spv_pool::{MAX_SPV_POOL_WITNESS, SpvPoolWitness};
            let program = bounded(&program, 32 * 1024 * 1024)?;
            let witness = SpvPoolWitness::from_bytes(&bounded(&witness, MAX_SPV_POOL_WITNESS)?)?;
            let receipts = receipts
                .iter()
                .map(|path| read_receipt(path))
                .collect::<Result<Vec<_>>>()?;
            let dependencies = if prove || !receipts.is_empty() || witness.assumptions()?.is_empty()
            {
                Dependencies::Genuine(&receipts)
            } else {
                Dependencies::Unresolved
            };
            run(&program, &witness, dependencies, &out, prove)
        }
        Command::Checkpoint {
            headers: input,
            height,
            work,
            out,
        } => {
            let context = zutti_spv_prototype::bootstrap::headers(&input)?;
            let work: [u8; 32] = hex::decode(work)?
                .try_into()
                .map_err(|_| "checkpoint work must be 32 bytes")?;
            let policy = Policy::new(Digest([1; 32]), NetworkType::Test, 1, 128, height)?;
            let checkpoint = HeaderClaim::checkpoint_context(policy, height, &context, work)?;
            if out.try_exists()? {
                return Err("checkpoint output already exists".into());
            }
            fs::create_dir(&out)?;
            write_new(&out.join("checkpoint.bin"), &checkpoint.to_bytes()?)?;
            let summary = serde_json::json!({"network":"testnet", "bootstrap":"checkpoint", "checkpoint_id":checkpoint.checkpoint_id()?.to_string(), "height":checkpoint.height()?, "hash":checkpoint.tip()?.to_string(), "work":hex::encode(work), "context_headers":context.len(), "proof_generated":false,"settlement_authority":false});
            write_new(
                &out.join("checkpoint.json"),
                &serde_json::to_vec_pretty(&summary)?,
            )?;
            println!("{summary}");
            Ok(())
        }
        Command::Verify {
            program,
            receipt,
            journal,
        } => {
            let program = bounded(&program, 32 * 1024 * 1024)?;
            let receipt = read_receipt(&receipt)?;
            verify(&receipt, image(&program)?, &bounded(&journal, 16_384)?)?;
            println!(
                "{}",
                serde_json::json!({"proof_verified":true,"image_id":image(&program)?.to_string(),"settlement_authority":false})
            );
            Ok(())
        }
        Command::Audit {
            program,
            receipt,
            out,
        } => {
            let program = bounded(&program, 32 * 1024 * 1024)?;
            let receipt = read_receipt(&receipt)?;
            let report = audit(&program, &receipt)?;
            write_new(&out, &serde_json::to_vec_pretty(&report)?)?;
            println!("{report}");
            Ok(())
        }
    }
}
