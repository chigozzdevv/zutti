mod args;
use args::{Args, BlockFormat, Command, Network};
use clap::Parser;
use std::{fs::File, io::Read, process::ExitCode};
use zcash_protocol::consensus::{MAIN_NETWORK, NetworkType, TEST_NETWORK};
use zutti_source::{
    block::{MAX_BLOCK_BYTES, ParsedBlock},
    work::verify_work,
};

fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        Command::Status => println!(
            "candidate_index=implemented block_parser=implemented equihash=implemented genesis_identity=implemented consensus_verifier=unavailable genesis_bootstrap=unavailable"
        ),
        Command::Inspect {
            network,
            block,
            format,
        } => {
            let mut bytes = Vec::new();
            let max = match format {
                BlockFormat::Raw => MAX_BLOCK_BYTES,
                BlockFormat::Hex => MAX_BLOCK_BYTES * 2 + 1024,
            };
            File::open(block)?
                .take(max as u64 + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() > max {
                return Err("block input exceeds its size limit".into());
            }
            let bytes = match format {
                BlockFormat::Raw => bytes,
                BlockFormat::Hex => {
                    let text = std::str::from_utf8(&bytes)?;
                    hex::decode(text.trim())?
                }
            };
            let block = match network {
                Network::Mainnet => ParsedBlock::parse(&bytes, &MAIN_NETWORK),
                Network::Testnet => ParsedBlock::parse(&bytes, &TEST_NETWORK),
            }?;
            println!(
                "{}",
                serde_json::json!({"claimed_height":block.claimed_height(),"block_hash_internal_hex":block.hash().to_string(),"transactions":block.txids().len(),"transaction_versions":block.versions(),"auth_data_root_internal_hex":block.auth_data_root().to_string(),"authorizing_commitment_verified":false,"work_verified": verify_work(&block, match network { Network::Mainnet => NetworkType::Main, Network::Testnet => NetworkType::Test }).is_ok(),"consensus_verified":false})
            );
        }
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
