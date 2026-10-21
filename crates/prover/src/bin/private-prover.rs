use clap::{Args as ClapArgs, Parser, Subcommand};
use std::{
    io,
    path::PathBuf,
    process::{Command as Process, ExitCode, Stdio},
};
use zutti_protocol::statement::ProofKind;
use zutti_protocol::{Digest, Secret32, encoding::Canonical};
use zutti_prover::{
    artifact::ProofArtifact,
    preparation::preparation_image_id,
    program::{Program, ProgramKind},
    query::{LocalProver, query_image_id},
};
use zutti_source::query::{MAX_QUERY_WITNESS, QueryWitness};
use zutti_wallet::backup;
use zutti_wallet::preparation::{MAX_PREPARATION_WITNESS, PreparationWitness};

#[derive(Debug, Parser)]
#[command(
    name = "private-prover",
    about = "Prove and verify private Zutti component claims locally"
)]
struct Args {
    #[command(subcommand)]
    command: Command,
}
#[derive(Debug, Subcommand)]
enum Command {
    Status,
    ExportPrograms {
        #[arg(long)]
        out: PathBuf,
    },
    InspectProgram {
        #[arg(long, value_enum)]
        kind: ProgramKind,
        #[arg(long)]
        program: PathBuf,
    },
    ProveQuery(ProveArgs),
    ProvePreparation(ProveArgs),
    ProvePool(ProveArgs),
    ExecutePool {
        #[arg(long)]
        witness: PathBuf,
        #[arg(long)]
        program: Option<PathBuf>,
        #[arg(long, default_value = "r0vm")]
        prover: PathBuf,
    },
    ExportExecution(ExecutionArgs),
    VerifyPool {
        #[arg(long)]
        proof: PathBuf,
    },
    VerifyQuery {
        #[arg(long)]
        proof: PathBuf,
    },
    VerifyPreparation {
        #[arg(long)]
        proof: PathBuf,
    },
    ImportBoundless {
        #[arg(long)]
        result: PathBuf,
        #[arg(long)]
        statement: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
}
#[derive(Debug, ClapArgs)]
struct ProveArgs {
    #[arg(long)]
    backup: PathBuf,
    #[arg(long)]
    key_file: PathBuf,
    #[arg(long)]
    domain: String,
    #[arg(long)]
    out: PathBuf,
    #[arg(long, default_value = "r0vm")]
    prover: PathBuf,
    #[arg(long)]
    program: Option<PathBuf>,
    #[arg(long, hide = true)]
    worker: bool,
}

#[derive(Debug, ClapArgs)]
struct ExecutionArgs {
    #[arg(long)]
    backup: PathBuf,
    #[arg(long)]
    key_file: PathBuf,
    #[arg(long)]
    domain: String,
    #[arg(long)]
    proof: PathBuf,
    #[arg(long)]
    out: PathBuf,
    #[arg(long)]
    program: Option<PathBuf>,
    #[arg(long)]
    preparation_proof: Option<PathBuf>,
    #[arg(long, requires = "preparation_proof")]
    preparation_program: Option<PathBuf>,
}

fn run_worker() -> Result<(), Box<dyn std::error::Error>> {
    let mut child = Process::new(std::env::current_exe()?)
        .args(std::env::args_os().skip(1))
        .arg("--worker")
        .stdout(Stdio::piped())
        .spawn()?;
    let output = child.stdout.take().ok_or("missing prover output stream")?;
    if let Err(error) = io::copy(&mut io::BufReader::new(output), &mut io::stderr().lock()) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error.into());
    }
    if !child.wait()?.success() {
        return Err("private proof worker failed".into());
    }
    Ok(())
}

fn report(artifact: &ProofArtifact, kind: ProofKind) -> Result<(), Box<dyn std::error::Error>> {
    if artifact.statement().kind() != kind {
        return Err("unexpected proof kind".into());
    }
    artifact.verify(artifact.statement())?;
    let name = match kind {
        ProofKind::HistoryQuery => "history_query",
        ProofKind::Preparation => "preparation",
        ProofKind::SourceTransition => "source_transition",
        ProofKind::SourceRange => "source_range",
        ProofKind::Source => "source_update",
        ProofKind::Note => "private_note",
        ProofKind::Reservation => "reservation",
        ProofKind::Settlement => "settlement",
    };
    println!(
        "{}",
        serde_json::json!({"proof_verified": true, "kind": name, "image_id": artifact.statement().program().to_string(), "consensus_verified": false, "settlement_authority": false})
    );
    Ok(())
}

fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        Command::ExportPrograms { out } => {
            std::fs::create_dir(&out)?;
            let mut images = serde_json::Map::new();
            for kind in [
                ProgramKind::Source,
                ProgramKind::SourceUpdate,
                ProgramKind::Pool,
                ProgramKind::Preparation,
                ProgramKind::HistoryQuery,
            ] {
                let program = Program::current(kind);
                program.export_new(&out.join(kind.name()))?;
                images.insert(kind.name().to_string(), program.image().to_string().into());
            }
            backup::persist_new(
                &out.join("programs.json"),
                &serde_json::to_vec_pretty(&serde_json::json!({"version":1,"images":images}))?,
            )?;
            println!("{}", serde_json::json!({"programs":out,"images":images}));
        }
        Command::InspectProgram { kind, program } => {
            let program = Program::read(kind, &program)?;
            println!(
                "{}",
                serde_json::json!({"kind":kind.name(),"image_id":program.image().to_string(),"program_verified":true})
            );
        }
        Command::Status => println!(
            "{}",
            serde_json::json!({"history_query": "implemented", "preparation": "implemented", "proving": "local", "image_id": query_image_id().to_string(), "preparation_image_id": preparation_image_id().to_string(), "pool_image_id": zutti_prover::pool::pool_image_id().to_string(), "settlement_authority": false})
        ),
        Command::VerifyQuery { proof } => {
            report(&ProofArtifact::read(&proof)?, ProofKind::HistoryQuery)?
        }
        Command::VerifyPreparation { proof } => {
            report(&ProofArtifact::read(&proof)?, ProofKind::Preparation)?
        }
        Command::ExecutePool {
            witness,
            prover,
            program,
        } => {
            let bytes = backup::read_private(&witness, zutti_wallet::pool::MAX_POOL_WITNESS)?;
            let witness = zutti_wallet::pool::PoolWitness::from_bytes(&bytes)?;
            println!(
                "{}",
                serde_json::to_string(&LocalProver::new(prover).execute_pool_with(
                    &witness,
                    &match program {
                        Some(path) => Program::read(ProgramKind::Pool, &path)?,
                        None => Program::current(ProgramKind::Pool),
                    }
                )?)?
            );
        }
        Command::VerifyPool { proof } => {
            let artifact = ProofArtifact::read(&proof)?;
            let kind = artifact.statement().kind();
            if !matches!(
                kind,
                ProofKind::Note | ProofKind::Reservation | ProofKind::Settlement
            ) {
                return Err("expected a pool proof".into());
            }
            report(&artifact, kind)?;
        }
        Command::ExportExecution(args) => export_execution(args)?,
        Command::ProvePool(args) => prove(args, ProofKind::Note)?,
        Command::ProveQuery(args) => prove(args, ProofKind::HistoryQuery)?,
        Command::ProvePreparation(args) => prove(args, ProofKind::Preparation)?,
        Command::ImportBoundless {
            result,
            statement,
            out,
        } => {
            let expected = zutti_protocol::statement::Statement::from_bytes(
                &backup::read_private(&statement, 65_664)?,
            )?;
            let artifact = zutti_prover::remote::import_groth16(&result, &expected)?;
            artifact.persist_new(&out)?;
            report(&artifact, expected.kind())?;
        }
    }
    Ok(())
}

fn export_execution(args: ExecutionArgs) -> Result<(), Box<dyn std::error::Error>> {
    use zutti_prover::submission::{PoolExecution, PoolOperation};
    let domain = Digest::from_hex(&args.domain)?;
    let key = backup::read_private(&args.key_file, 32)?;
    let key = Secret32::from_bytes(
        key.as_slice()
            .try_into()
            .map_err(|_| "key file must contain exactly 32 raw bytes")?,
    )?;
    let archive = backup::read_private(&args.backup, zutti_wallet::pool::MAX_POOL_WITNESS + 128)?;
    let plaintext = backup::open(domain, &archive, &key)?;
    let witness = zutti_wallet::pool::PoolWitness::from_bytes(&plaintext)?;
    let operation = PoolOperation::from_witness(&witness)?;
    if operation.claim().domain != domain {
        return Err("pool operation domain mismatch".into());
    }
    let program = match args.program {
        Some(path) => Program::read(ProgramKind::Pool, &path)?,
        None => Program::current(ProgramKind::Pool),
    };
    let artifact = ProofArtifact::read(&args.proof)?;
    let preparation = match args.preparation_proof {
        Some(path) => Some((
            match args.preparation_program {
                Some(path) => Program::read(ProgramKind::Preparation, &path)?,
                None => Program::current(ProgramKind::Preparation),
            },
            ProofArtifact::read(&path)?,
        )),
        None => None,
    };
    let expected = zutti_prover::pool::pool_statement_with(operation.claim(), &program)?;
    let preparation = preparation
        .map(|(program, artifact)| {
            let claim = zutti_wallet::preparation::PreparationClaim::from_bytes(
                artifact.statement().payload(),
            )?;
            let expected = zutti_prover::preparation::preparation_statement_with(&claim, &program)?;
            Ok::<_, zutti_prover::Error>((expected, artifact))
        })
        .transpose()?;
    let execution = PoolExecution::verify(
        operation,
        &expected,
        &artifact,
        preparation.as_ref().map(|(s, a)| (s, a)),
    )?;
    execution.persist_new(&args.out)?;
    println!("proof_verified=true chain_admission_verified=false transaction_submitted=false");
    Ok(())
}

fn prove(args: ProveArgs, kind: ProofKind) -> Result<(), Box<dyn std::error::Error>> {
    let ProveArgs {
        backup: path,
        key_file,
        domain,
        out,
        prover,
        program,
        worker,
    } = args;
    if out.try_exists()? {
        return Err("proof output already exists".into());
    }
    if !worker {
        run_worker()?;
        let artifact = ProofArtifact::read(&out)?;
        let expected = if kind == ProofKind::Note {
            artifact.statement().kind()
        } else {
            kind
        };
        if kind == ProofKind::Note
            && !matches!(
                expected,
                ProofKind::Note | ProofKind::Reservation | ProofKind::Settlement
            )
        {
            return Err("expected a pool proof".into());
        }
        return report(&artifact, expected);
    }
    let domain = Digest::from_hex(&domain)?;
    let key = backup::read_private(&key_file, 32)?;
    let key = Secret32::from_bytes(
        key.as_slice()
            .try_into()
            .map_err(|_| "key file must contain exactly 32 raw bytes")?,
    )?;
    let archive = backup::read_private(&path, backup::MAX_ARCHIVE)?;
    let plaintext = backup::open(domain, &archive, &key)?;
    let prover = LocalProver::new(prover);
    let program_kind = match kind {
        ProofKind::HistoryQuery => ProgramKind::HistoryQuery,
        ProofKind::Preparation => ProgramKind::Preparation,
        ProofKind::Note => ProgramKind::Pool,
        _ => return Err("unsupported private proof kind".into()),
    };
    let program = match program {
        Some(path) => Program::read(program_kind, &path)?,
        None => Program::current(program_kind),
    };
    let (statement, proof) = match kind {
        ProofKind::HistoryQuery => {
            if plaintext.len() > MAX_QUERY_WITNESS {
                return Err("query witness exceeds its size limit".into());
            }
            let witness = QueryWitness::from_bytes(&plaintext)?;
            if witness.evaluate()?.domain() != domain {
                return Err("query witness domain mismatch".into());
            }
            prover.prove_query_with(&witness, &program)?
        }
        ProofKind::Preparation => {
            if plaintext.len() > MAX_PREPARATION_WITNESS {
                return Err("preparation witness exceeds its size limit".into());
            }
            let witness = PreparationWitness::from_bytes(&plaintext)?;
            if witness.evaluate()?.domain() != domain {
                return Err("preparation witness domain mismatch".into());
            }
            prover.prove_preparation_with(&witness, &program)?
        }
        ProofKind::Note => {
            if plaintext.len() > zutti_wallet::pool::MAX_POOL_WITNESS {
                return Err("pool witness exceeds its size limit".into());
            }
            let witness = zutti_wallet::pool::PoolWitness::from_bytes(&plaintext)?;
            if witness.evaluate()?.domain != domain {
                return Err("pool witness domain mismatch".into());
            }
            prover.prove_pool_with(&witness, &program)?
        }
        _ => return Err("unsupported proof kind".into()),
    };
    let artifact = ProofArtifact::new(statement, proof)?;
    artifact.persist_new(&out)?;
    Ok(())
}

fn main() -> ExitCode {
    match run(Args::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(2)
        }
    }
}
