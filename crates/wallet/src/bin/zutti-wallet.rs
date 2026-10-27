use clap::{Parser, Subcommand};
use std::path::PathBuf;
use zutti_wallet::account::WalletAccount;

#[derive(Parser)]
#[command(name = "zutti-wallet")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Init {
        #[arg(long)]
        directory: PathBuf,
    },
    Address {
        #[arg(long)]
        directory: PathBuf,
    },
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let account = match Args::parse().command {
        Command::Init { directory } => WalletAccount::create(&directory)?,
        Command::Address { directory } => WalletAccount::open(&directory)?,
    };
    let recipient = account.private_receiver()?;
    println!(
        "{}",
        serde_json::json!({
            "version": 1,
            "network": "testnet",
            "zcash_address": account.zcash_address()?,
            "usdc_receiver": {
                "owner": hex_bytes(recipient.owner()),
                "viewing": hex_bytes(recipient.viewing()),
                "nullifier_key_commitment": recipient.nullifier_key().to_string()
            }
        })
    );
    Ok(())
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
