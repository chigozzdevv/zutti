mod args;
use args::{Args, Command};
use clap::Parser;
use std::process::ExitCode;
use zutti_protocol::{Digest, Secret32};
use zutti_wallet::backup;

fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        Command::Status => println!("encrypted_archives=implemented claim=unavailable refund=unavailable"),
        Command::Inspect { backup: path, key_file, domain } => {
            let domain = Digest::from_hex(&domain)?;
            let key = backup::read_private(&key_file,32)?;
            let key_bytes: [u8;32] = key.as_slice().try_into().map_err(|_| "key file must contain exactly 32 raw bytes")?;
            let key = Secret32::from_bytes(key_bytes)?;
            let archive = backup::read_private(&path,backup::MAX_ARCHIVE)?;
            let plaintext = backup::open(domain,&archive,&key)?;
            println!("archive_authenticated=true payload_bytes={} settlement_authority=false",plaintext.len());
        },
        Command::Claim | Command::Refund => return Err("settlement unavailable: source consensus, pool/settlement proofs, Ethereum finality and contracts have not been admitted; no transaction submitted".into()),
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
