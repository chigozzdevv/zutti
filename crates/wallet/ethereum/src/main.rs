use clap::{Parser, Subcommand};
use std::{
    io::Read,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use zutti_ethereum::{
    ConsensusConfig, Evidence, FinalityEvidence, ReservationRequest, verify, verify_finalized,
};

#[derive(Debug, Parser)]
struct Args {
    #[command(subcommand)]
    command: Command,
}
#[derive(Debug, Subcommand)]
enum Command {
    Inventory {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        request: PathBuf,
        #[arg(long)]
        evidence: PathBuf,
    },
    Finality {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        evidence: PathBuf,
    },
    Reservation {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        request: PathBuf,
        #[arg(long)]
        evidence: PathBuf,
    },
}
fn read<T: serde::de::DeserializeOwned>(
    path: &Path,
    limit: u64,
) -> Result<T, Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err("verification input exceeds its bound".into());
    }
    Ok(serde_json::from_slice(&bytes)?)
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    match Args::parse().command {
        Command::Inventory {
            config,
            request,
            evidence,
        } => {
            use zutti_ethereum::inventory::{InventoryEvidence, InventoryRequest, verify};
            println!(
                "{}",
                serde_json::to_string(&verify(
                    &read::<ConsensusConfig>(&config, 65_536)?,
                    &read::<InventoryRequest>(&request, 16_384)?,
                    &read::<InventoryEvidence>(&evidence, 16_777_216)?,
                    now,
                )?)?
            );
        }
        Command::Finality { config, evidence } => {
            let config: ConsensusConfig = read(&config, 65_536)?;
            let evidence: FinalityEvidence = read(&evidence, 16_777_216)?;
            println!(
                "{}",
                serde_json::to_string(&verify_finalized(&config, &evidence, now)?)?
            );
        }
        Command::Reservation {
            config,
            request,
            evidence,
        } => {
            let config: ConsensusConfig = read(&config, 65_536)?;
            let request: ReservationRequest = read(&request, 16_384)?;
            let evidence: Evidence = read(&evidence, 16_777_216)?;
            println!(
                "{}",
                serde_json::to_string(&verify(&config, &request, &evidence, now)?)?
            );
        }
    }
    Ok(())
}
fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
