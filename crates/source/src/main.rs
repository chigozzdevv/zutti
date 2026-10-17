mod args;
use args::{Args, BlockFormat, Command, Network};
use clap::Parser;
use std::{fs::File, io::Read, path::Path, process::ExitCode};
use zcash_protocol::consensus::{MAIN_NETWORK, NetworkType, TEST_NETWORK};
use zutti_protocol::Digest;
use zutti_source::{
    block::{MAX_BLOCK_BYTES, ParsedBlock},
    bootstrap::GenesisBlock,
    state::SourceCandidate,
    work::verify_work,
};

fn read_block(path: &Path, format: BlockFormat) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let max = match format {
        BlockFormat::Raw => MAX_BLOCK_BYTES,
        BlockFormat::Hex => MAX_BLOCK_BYTES * 2 + 1024,
    };
    let file = File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err("block input must be a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(max as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > max {
        return Err("block input exceeds its size limit".into());
    }
    Ok(match format {
        BlockFormat::Raw => bytes,
        BlockFormat::Hex => hex::decode(std::str::from_utf8(&bytes)?.trim())?,
    })
}

fn parse(bytes: &[u8], network: Network) -> zutti_source::Result<ParsedBlock> {
    match network {
        Network::Mainnet => ParsedBlock::parse(bytes, &MAIN_NETWORK),
        Network::Testnet => ParsedBlock::parse(bytes, &TEST_NETWORK),
    }
}

fn network_type(network: Network) -> NetworkType {
    match network {
        Network::Mainnet => NetworkType::Main,
        Network::Testnet => NetworkType::Test,
    }
}

fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        Command::Status => println!(
            "candidate_index=implemented block_parser=implemented equihash=implemented genesis_identity=implemented chain_history=implemented commitment_trees=implemented candidate_replay=implemented consensus_verifier=unavailable genesis_bootstrap=unavailable"
        ),
        Command::Inspect {
            network,
            block,
            format,
        } => {
            let block = parse(&read_block(&block, format)?, network)?;
            println!(
                "{}",
                serde_json::json!({"claimed_height":block.claimed_height(),"block_hash_internal_hex":block.hash().to_string(),"transactions":block.txids().len(),"transaction_versions":block.versions(),"auth_data_root_internal_hex":block.auth_data_root().to_string(),"authorizing_commitment_verified":false,"work_verified": verify_work(&block, network_type(network)).is_ok(),"consensus_verified":false})
            );
        }
        Command::Replay {
            network,
            genesis,
            blocks,
            rules,
            index_start,
            format,
        } => {
            let genesis =
                GenesisBlock::parse(&read_block(&genesis, format)?, network_type(network))?;
            let mut source =
                SourceCandidate::from_genesis(&genesis, Digest::from_hex(&rules)?, index_start)?;
            for path in &blocks {
                source.append(&parse(&read_block(path, format)?, network)?)?;
            }
            let roots = source.tree_roots();
            let index = match source.index_snapshot() {
                Ok(index) => Some(index),
                Err(zutti_source::state::TransitionError::IndexUnavailable) => None,
                Err(error) => return Err(error.into()),
            };
            println!(
                "{}",
                serde_json::json!({
                    "candidate_height":source.height()?, "block_hash_internal_hex":source.tip()?.to_string(),
                    "cumulative_work_be_hex":hex::encode(source.cumulative_work()?),
                    "sprout_root_internal_hex":source.sprout_root().to_string(),
                    "sapling_root_internal_hex":roots.sapling.to_string(),
                    "orchard_root_internal_hex":roots.orchard.to_string(),
                    "ironwood_root_internal_hex":roots.ironwood.to_string(),
                    "chain_history_root_internal_hex":source.history_root()?.map(|r|r.to_string()),
                    "index_ready":index.is_some(), "index_height":index.map(|s|s.height),
                    "index_root":index.map(|s|s.root.to_string()), "processed_blocks":blocks.len()+1,
                    "consensus_verified":false,"settlement_authority":false
                })
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
