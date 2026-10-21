use clap::{Args as ClapArgs, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "zutti-recovery",
    about = "Restore encrypted swaps and prepare independently verifiable settlement witnesses"
)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}
#[derive(Debug, Subcommand)]
pub enum Command {
    Status,
    Inspect {
        #[arg(long)]
        backup: PathBuf,
        #[arg(long)]
        key_file: PathBuf,
        #[arg(long)]
        domain: String,
    },
    Export(ExportArgs),
    Restore(RestoreArgs),
    PrepareClaim(PrepareArgs),
    PrepareRefund(PrepareArgs),
    Claim,
    Refund,
}

#[derive(Debug, ClapArgs)]
pub struct ArchiveArgs {
    #[arg(long)]
    pub backup: PathBuf,
    #[arg(long)]
    pub key_file: PathBuf,
    #[arg(long)]
    pub domain: String,
}

#[derive(Debug, ClapArgs)]
pub struct RecordArgs {
    #[command(flatten)]
    pub archive: ArchiveArgs,
    #[arg(long)]
    pub swap: String,
}

#[derive(Debug, ClapArgs)]
pub struct ExportArgs {
    #[command(flatten)]
    pub archive: ArchiveArgs,
    #[arg(long)]
    pub out: PathBuf,
}

#[derive(Debug, ClapArgs)]
pub struct RestoreArgs {
    #[command(flatten)]
    pub record: RecordArgs,
    #[arg(long)]
    pub out: PathBuf,
}

#[derive(Debug, ClapArgs)]
pub struct PrepareArgs {
    #[command(flatten)]
    pub record: RecordArgs,
    #[arg(long)]
    pub evidence: PathBuf,
    #[arg(long)]
    pub out: PathBuf,
}
