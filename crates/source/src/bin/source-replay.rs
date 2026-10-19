use clap::{Parser, ValueEnum};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::Instant,
};
use zcash_protocol::consensus::NetworkType;
use zutti_protocol::{Digest, encoding::Canonical};
use zutti_source::{
    block::MAX_BLOCK_BYTES,
    proof::SourcePolicy,
    state::TransitionError,
    storage::{Error as StorageError, ReplayStore},
    transition::MAX_BLOCKS,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
#[derive(Clone, Copy, Debug, ValueEnum)]
enum Network {
    Mainnet,
    Testnet,
}
#[derive(Debug, Parser)]
struct Args {
    #[arg(long, value_enum)]
    network: Network,
    #[arg(long)]
    database: PathBuf,
    #[arg(long)]
    genesis: PathBuf,
    #[arg(long)]
    source_program: String,
    #[arg(long)]
    registry_program: String,
    #[arg(long, default_value_t = 0)]
    index_start: u32,
    #[arg(long, default_value_t = 100)]
    confirmations: u32,
    #[arg(long)]
    through_height: u32,
    #[arg(long, default_value_t = 128, value_parser = clap::value_parser!(u8).range(1..=MAX_BLOCKS as i64))]
    batch_size: u8,
    #[arg(long)]
    sample_start: Vec<u32>,
    #[arg(long, default_value_t = 16, value_parser = clap::value_parser!(u8).range(1..=MAX_BLOCKS as i64))]
    sample_size: u8,
    #[arg(long)]
    samples: PathBuf,
}
fn save(path: &Path, bytes: &[u8]) -> Result<()> {
    if path.try_exists()? {
        if read(path, bytes.len())? != bytes {
            return Err("retained sample differs from replay".into());
        }
        return Ok(());
    }
    let parent = path.parent().ok_or("sample requires a parent directory")?;
    let mut out = tempfile::NamedTempFile::new_in(parent)?;
    out.write_all(bytes)?;
    out.as_file().sync_all()?;
    out.persist_noclobber(path).map_err(|e| e.error)?;
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}
fn read(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let file = fs::File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err("input must be a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err("input exceeds its size bound".into());
    }
    Ok(bytes)
}
fn block(input: &mut impl Read) -> Result<Vec<u8>> {
    let mut length = [0; 4];
    input.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    if length == 0 || length > MAX_BLOCK_BYTES {
        return Err("invalid block frame length".into());
    }
    let mut bytes = vec![0; length];
    input.read_exact(&mut bytes)?;
    Ok(bytes)
}
fn advance(store: &mut ReplayStore, blocks: Vec<Vec<u8>>) -> Result<()> {
    match store.advance(blocks.clone()) {
        Ok(_) => Ok(()),
        Err(StorageError::Source(TransitionError::WitnessSize { bytes, limit }))
            if blocks.len() > 1 =>
        {
            eprintln!(
                "{}",
                serde_json::json!({"stage":"split","start_height":store.candidate().height()?,
                "blocks":blocks.len(),"witness_bytes":bytes,"limit":limit})
            );
            let mut first = blocks;
            let second = first.split_off(first.len() / 2);
            advance(store, first)?;
            advance(store, second)
        }
        Err(error) => Err(error.into()),
    }
}
fn export(store: &ReplayStore, start: u32, out: &Path) -> Result<()> {
    let witness = store
        .witness(start)?
        .ok_or("retained replay sample is missing")?;
    let claim = witness.evaluate()?;
    let dir = out.join(format!("{}-{}", claim.start.height, claim.end.height));
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)?;
    save(&dir.join("witness.bin"), &witness.to_bytes()?)?;
    save(&dir.join("claim.bin"), &claim.to_bytes()?)?;
    let report = serde_json::json!({"start_height":claim.start.height,"end_height":claim.end.height,
        "program":claim.end.program.to_string(),"policy":claim.end.policy.to_string(),
        "source_state":claim.start.state.to_string(),"result_state":claim.end.state.to_string(),
        "native_replay_from_genesis":true,"genesis_anchored_proof":false,
        "proof_generated":false,"current_chain_admitted":false,"settlement_authority":false});
    save(
        &dir.join("sample.json"),
        &serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}
fn run(mut args: Args) -> Result<()> {
    args.sample_start.sort_unstable();
    args.sample_start.dedup();
    if args.sample_start.len() > 32
        || args.sample_start.iter().any(|h| {
            h.checked_add(u32::from(args.sample_size))
                .is_none_or(|end| end > args.through_height)
        })
        || args
            .sample_start
            .windows(2)
            .any(|p| p[0] + u32::from(args.sample_size) > p[1])
    {
        return Err("invalid or overlapping replay sample intervals".into());
    }
    let policy = SourcePolicy {
        program: Digest::from_hex(&args.source_program)?,
        registry_program: Digest::from_hex(&args.registry_program)?,
        network: match args.network {
            Network::Mainnet => NetworkType::Main,
            Network::Testnet => NetworkType::Test,
        },
        index_start: args.index_start,
        confirmations: args.confirmations,
    };
    let mut store = if args.database.try_exists()? {
        ReplayStore::open(&args.database, policy)?
    } else {
        ReplayStore::create(
            &args.database,
            &read(&args.genesis, MAX_BLOCK_BYTES)?,
            policy,
        )?
    };
    let retained = store.samples()?;
    for (start, _) in &retained {
        export(&store, *start, &args.samples)?;
    }
    let initial = store.candidate().height()?;
    if initial > args.through_height
        || args
            .sample_start
            .iter()
            .any(|h| *h < initial && !retained.iter().any(|(start, _)| start == h))
    {
        return Err("requested sample precedes the retained replay frontier".into());
    }
    println!(
        "{}",
        serde_json::json!({"stage":"ready","native_height":initial,
        "tip":store.candidate().tip()?.to_string(),"policy":policy.id()?.to_string(),
        "genesis_anchored_proof":false,"current_chain_admitted":false})
    );
    io::stdout().flush()?;
    let mut input = io::stdin().lock();
    while store.candidate().height()? < args.through_height {
        let height = store.candidate().height()?;
        let sample = args.sample_start.binary_search(&height).is_ok();
        let boundary = args
            .sample_start
            .iter()
            .copied()
            .find(|h| *h > height)
            .unwrap_or(args.through_height);
        let count = u32::from(if sample {
            args.sample_size
        } else {
            args.batch_size
        });
        let end = height
            .checked_add(count)
            .ok_or("replay height overflow")?
            .min(boundary)
            .min(args.through_height);
        let blocks = (height..end)
            .map(|_| block(&mut input))
            .collect::<Result<Vec<_>>>()?;
        let bytes = blocks.iter().map(Vec::len).sum::<usize>();
        let started = Instant::now();
        let result = if sample {
            store.sample(blocks).map(|_| ()).map_err(Into::into)
        } else {
            advance(&mut store, blocks)
        };
        result.map_err(|error: Box<dyn std::error::Error>| {
            format!(
                "source replay {}..={end} ({bytes} block bytes): {error}",
                height + 1
            )
        })?;
        if sample {
            export(&store, height, &args.samples)?;
        }
        eprintln!(
            "{}",
            serde_json::json!({"stage":"replayed","start_height":height,
            "end_height":store.candidate().height()?,"block_bytes":bytes,"seconds":started.elapsed().as_secs_f64(),
            "sample_retained":sample,"genesis_anchored_proof":false,"current_chain_admitted":false})
        );
    }
    println!(
        "{}",
        serde_json::json!({"stage":"complete","native_height":store.candidate().height()?,
        "samples":store.samples()?,"genesis_anchored_proof":false,"current_chain_admitted":false})
    );
    Ok(())
}
fn main() -> std::process::ExitCode {
    match run(Args::parse()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
