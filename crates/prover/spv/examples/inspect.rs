use std::{env, path::Path};
use zcash_protocol::consensus::NetworkType;
use zutti_source::{spv::parse_block, work::verify_header_work};
use zutti_spv_prototype::{Result, bounded};

fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let network = match args.next().as_deref() {
        Some("testnet") => NetworkType::Test,
        Some("mainnet") => NetworkType::Main,
        _ => return Err("expected mainnet or testnet followed by block paths".into()),
    };
    for path in args {
        let raw = bounded(Path::new(&path), 4_000_001)?;
        let raw = if path.ends_with(".txt") || path.ends_with(".hex") {
            hex::decode(std::str::from_utf8(&raw)?.trim())?
        } else {
            raw
        };
        let block = parse_block(&raw, network)?;
        verify_header_work(block.header(), network)?;
        let mut counts = [0_u32; 4];
        let mut first_shielded = None;
        for (index, tx) in block.transactions().iter().enumerate().skip(1) {
            let pools = [
                tx.sprout_bundle().is_some(),
                tx.sapling_bundle().is_some(),
                tx.orchard_bundle().is_some(),
                tx.ironwood_bundle().is_some(),
            ];
            for (count, present) in counts.iter_mut().zip(pools) {
                *count += u32::from(present);
            }
            if first_shielded.is_none() && pools.into_iter().any(|p| p) {
                first_shielded = Some(
                    serde_json::json!({"index":index,"txid":block.txids()[index].to_string()}),
                );
            }
        }
        println!(
            "{}",
            serde_json::json!({"path":path,"height":block.claimed_height(),
            "bytes":raw.len(),"hash":block.hash().to_string(),"transactions":block.transactions().len(),
            "non_coinbase_pool_counts":{"sprout":counts[0],"sapling":counts[1],"orchard":counts[2],"ironwood":counts[3]},
            "first_shielded":first_shielded,"header_work_verified":true,"chain_authenticated":false})
        );
    }
    Ok(())
}
