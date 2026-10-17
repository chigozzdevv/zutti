use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "zutti-source",
    about = "Inspect candidate Zcash block data; no consensus acceptance"
)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Network {
    Mainnet,
    Testnet,
}
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum BlockFormat {
    Raw,
    Hex,
}
#[derive(Debug, Subcommand)]
pub enum Command {
    Status,
    Inspect {
        #[arg(long, value_enum)]
        network: Network,
        #[arg(long)]
        block: PathBuf,
        #[arg(long, value_enum, default_value = "raw")]
        format: BlockFormat,
    },
}
