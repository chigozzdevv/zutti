use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "zutti-recovery",
    about = "Authenticate encrypted archives; settlement is not admitted"
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
    Claim,
    Refund,
}
