mod archive;
mod args;
mod prepare;
mod restore;

use args::{ArchiveArgs, Args, Command};
use clap::Parser;
use std::process::ExitCode;
use zutti_wallet::{backup, swap::SettlementAction};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn run(args: Args) -> Result<()> {
    match args.command {
        Command::Status => println!(
            "encrypted_archives=implemented swap_restore=implemented settlement_witness=implemented claim=unavailable refund=unavailable"
        ),
        Command::Inspect { backup, key_file, domain } => {
            let (_, _, plaintext) = archive::open(
                &ArchiveArgs { backup, key_file, domain },
                backup::MAX_PLAINTEXT,
            )?;
            println!(
                "archive_authenticated=true payload_bytes={} settlement_authority=false",
                plaintext.len()
            );
        }
        Command::Export(args) => restore::export(args)?,
        Command::Restore(args) => restore::restore(args)?,
        Command::PrepareClaim(args) => prepare::prepare(args, SettlementAction::Claim)?,
        Command::PrepareRefund(args) => prepare::prepare(args, SettlementAction::Refund)?,
        Command::Claim | Command::Refund => return Err(
            "settlement submission requires admitted chain evidence, a verified settlement proof and a configured deployment; no transaction submitted".into(),
        ),
    }
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
