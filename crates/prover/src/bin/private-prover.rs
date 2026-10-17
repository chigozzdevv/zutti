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
    ProveQuery(ProveArgs),
    ProvePreparation(ProveArgs),
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
    #[arg(long, hide = true)]
    worker: bool,
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
        _ => return Err("unsupported proof kind".into()),
    };
    println!(
        "{}",
        serde_json::json!({"proof_verified": true, "kind": name, "image_id": artifact.statement().program().to_string(), "consensus_verified": false, "settlement_authority": false})
    );
    Ok(())
}

fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        Command::Status => println!(
            "{}",
            serde_json::json!({"history_query": "implemented", "preparation": "implemented", "proving": "local", "image_id": query_image_id().to_string(), "preparation_image_id": preparation_image_id().to_string(), "settlement_authority": false})
        ),
        Command::VerifyQuery { proof } => {
            report(&ProofArtifact::read(&proof)?, ProofKind::HistoryQuery)?
        }
        Command::VerifyPreparation { proof } => {
            report(&ProofArtifact::read(&proof)?, ProofKind::Preparation)?
        }
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

fn prove(args: ProveArgs, kind: ProofKind) -> Result<(), Box<dyn std::error::Error>> {
    let ProveArgs {
        backup: path,
        key_file,
        domain,
        out,
        prover,
        worker,
    } = args;
    if out.try_exists()? {
        return Err("proof output already exists".into());
    }
    if !worker {
        run_worker()?;
        return report(&ProofArtifact::read(&out)?, kind);
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
    let (statement, proof) = match kind {
        ProofKind::HistoryQuery => {
            if plaintext.len() > MAX_QUERY_WITNESS {
                return Err("query witness exceeds its size limit".into());
            }
            let witness = QueryWitness::from_bytes(&plaintext)?;
            if witness.evaluate()?.domain() != domain {
                return Err("query witness domain mismatch".into());
            }
            prover.prove_query(&witness)?
        }
        ProofKind::Preparation => {
            if plaintext.len() > MAX_PREPARATION_WITNESS {
                return Err("preparation witness exceeds its size limit".into());
            }
            let witness = PreparationWitness::from_bytes(&plaintext)?;
            if witness.evaluate()?.domain() != domain {
                return Err("preparation witness domain mismatch".into());
            }
            prover.prove_preparation(&witness)?
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
