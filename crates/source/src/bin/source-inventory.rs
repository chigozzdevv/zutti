use clap::{Parser, ValueEnum};
use std::io::{self, BufWriter, Read, Write};
use zcash_protocol::consensus::{MAIN_NETWORK, TEST_NETWORK};
use zutti_protocol::Digest;
use zutti_source::{block::MAX_BLOCK_BYTES, inventory::InventoryChain};

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Network {
    Mainnet,
    Testnet,
}

#[derive(Debug, Parser)]
#[command(about = "Count public block workloads without generating consensus evidence")]
struct Args {
    #[arg(long, value_enum)]
    network: Network,
    #[arg(long)]
    start_height: u32,
    #[arg(long)]
    parent: String,
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..=10_000_000))]
    max_blocks: u32,
}

fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let mut parent = <[u8; 32]>::try_from(hex::decode(args.parent)?)
        .map_err(|_| "parent hash must contain 32 bytes")?;
    parent.reverse();
    let mut chain = InventoryChain::new(args.start_height, Digest(parent));
    let mut input = io::stdin().lock();
    let mut output = BufWriter::new(io::stdout().lock());
    for _ in 0..args.max_blocks {
        let mut length = [0; 4];
        if input.read(&mut length[..1])? == 0 {
            return Ok(());
        }
        input.read_exact(&mut length[1..])?;
        let length = u32::from_le_bytes(length) as usize;
        if length == 0 || length > MAX_BLOCK_BYTES {
            return Err("invalid block frame length".into());
        }
        let mut bytes = vec![0; length];
        input.read_exact(&mut bytes)?;
        let row = match args.network {
            Network::Mainnet => chain.append(&bytes, &MAIN_NETWORK),
            Network::Testnet => chain.append(&bytes, &TEST_NETWORK),
        }?;
        let mut hash = row.hash.0;
        let mut parent = row.parent.0;
        hash.reverse();
        parent.reverse();
        writeln!(
            output,
            "{}",
            serde_json::json!({
                "schema":1,"height":row.height,"block_hash":hex::encode(hash),
                "parent_hash":hex::encode(parent),"branch":format!("{:?}",row.branch),
                "branch_id":format!("{:08x}",u32::from(row.branch)),
                "block_bytes":row.bytes,"counts":row.counts,"versions":row.versions,
                "consensus_verified":false,"proof_generated":false
            })
        )?;
        output.flush()?;
    }
    Ok(())
}

fn main() -> std::process::ExitCode {
    match run(Args::parse()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("inventory failed: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
