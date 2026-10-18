use super::{Profile, block_bytes};
use clap::{Args, Subcommand};
use std::path::{Path, PathBuf};
use zutti_prover::{
    artifact::ProofArtifact,
    bootstrap::SourceBootstrap,
    database::MongoDatabase,
    local::LocalProver,
    rpc::SourceRpc,
    source::{MAX_SOURCE_CYCLES, SourceJob},
};
use zutti_source::{storage::RangeLimits, transition::MAX_BLOCKS};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
#[derive(Debug, Args)]
pub(super) struct Options {
    #[command(flatten)]
    profile: Profile,
    #[arg(long)]
    database: PathBuf,
    #[arg(long, default_value_t = 16, value_parser = clap::value_parser!(u8).range(1..=64))]
    queue_batches: u8,
    #[arg(long, default_value_t = 134_217_728, value_parser = clap::value_parser!(u32).range(1..=536_870_912))]
    queue_bytes: u32,
    #[command(subcommand)]
    command: Command,
}
#[derive(Debug, Args)]
pub(super) struct FillOptions {
    #[arg(long)]
    pub rpc: String,
    #[arg(long)]
    pub rpc_cookie: Option<PathBuf>,
    #[arg(long, default_value_t = 4, value_parser = clap::value_parser!(u8).range(1..=MAX_BLOCKS as i64))]
    pub batch_size: u8,
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=64))]
    pub batches: u8,
    #[arg(long)]
    pub through_height: Option<u32>,
    #[arg(long, default_value = "r0vm")]
    pub prover: PathBuf,
    #[arg(long, default_value_t = MAX_SOURCE_CYCLES, value_parser = clap::value_parser!(u64).range(1..=MAX_SOURCE_CYCLES))]
    pub cycle_limit: u64,
}
#[derive(Debug, Subcommand)]
enum Command {
    Init {
        #[arg(long)]
        genesis: PathBuf,
    },
    Migrate {
        #[arg(long)]
        from: PathBuf,
    },
    Status,
    Fill(FillOptions),
    Run(super::bootstrap_run::Options),
    Queue {
        #[arg(long, required = true)]
        block: Vec<PathBuf>,
        #[arg(long, default_value = "r0vm")]
        prover: PathBuf,
        #[arg(long, default_value_t = MAX_SOURCE_CYCLES, value_parser = clap::value_parser!(u64).range(1..=MAX_SOURCE_CYCLES))]
        cycle_limit: u64,
    },
    Export {
        #[command(subcommand)]
        job: Job,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value = "r0vm")]
        prover: PathBuf,
        #[arg(long, default_value_t = MAX_SOURCE_CYCLES, value_parser = clap::value_parser!(u64).range(1..=MAX_SOURCE_CYCLES))]
        cycle_limit: u64,
    },
    Accept {
        #[command(subcommand)]
        job: Acceptance,
        #[arg(long)]
        proof: PathBuf,
    },
}
#[derive(Debug, Subcommand)]
enum Job {
    Genesis,
    Range {
        #[arg(long)]
        batch: u64,
    },
    Continue {
        #[arg(long)]
        batch: u64,
    },
    Aggregate {
        #[arg(long, required = true)]
        batch: Vec<u64>,
    },
    Fold {
        #[arg(long)]
        batch: u64,
    },
}
#[derive(Debug, Subcommand)]
enum Acceptance {
    Prefix,
    Range {
        #[arg(long)]
        batch: u64,
    },
    Aggregate {
        #[arg(long, required = true)]
        batch: Vec<u64>,
    },
}
pub(super) fn run(options: Options) -> Result<()> {
    let programs = options.profile.programs()?;
    let policy = options.profile.policy(&programs);
    let limits = RangeLimits {
        batches: usize::from(options.queue_batches),
        witness_bytes: options.queue_bytes as usize,
    };
    let mongo = match std::env::var("ZUTTI_MONGODB_URI") {
        Ok(uri) => Some(MongoDatabase::connect(
            &uri,
            &std::env::var("ZUTTI_MONGODB_DATABASE").unwrap_or_else(|_| "zutti".into()),
            options
                .database
                .to_str()
                .ok_or("invalid source store name")?,
        )?),
        Err(std::env::VarError::NotPresent) => None,
        Err(_) => return Err("invalid MongoDB configuration".into()),
    };
    let cache = match &mongo {
        Some(_) => PathBuf::from(
            std::env::var_os("ZUTTI_CACHE_DIR")
                .ok_or("ZUTTI_CACHE_DIR is required for MongoDB source storage")?,
        )
        .join(&options.database)
        .with_extension("sqlite"),
        None => options.database.clone(),
    };
    if let Command::Migrate { from } = &options.command {
        let mongo = mongo
            .as_ref()
            .ok_or("MongoDB storage must be configured for migration")?;
        let bootstrap = SourceBootstrap::open(from, policy, programs.clone(), limits)?;
        mongo.initialize(bootstrap.store())?;
        drop(bootstrap);
        let restored = SourceBootstrap::from_store(mongo.open(&cache, policy, limits)?, programs)?;
        return status(&restored);
    }
    if let Command::Init { genesis } = &options.command {
        let temp = mongo.as_ref().map(|_| tempfile::tempdir()).transpose()?;
        let initial = temp.as_ref().map(|d| d.path().join("source.sqlite"));
        let bootstrap = SourceBootstrap::create(
            initial.as_deref().unwrap_or(&cache),
            block_bytes(genesis)?,
            policy,
            programs,
            limits,
        )?;
        if let Some(mongo) = mongo {
            mongo.initialize(bootstrap.store())?;
        }
        return status(&bootstrap);
    }
    let mut bootstrap = match mongo {
        Some(mongo) => SourceBootstrap::from_store(mongo.open(&cache, policy, limits)?, programs)?,
        None => SourceBootstrap::open(&cache, policy, programs, limits)?,
    };
    match options.command {
        Command::Init { .. } => return Err("source bootstrap is already initialized".into()),
        Command::Migrate { .. } => return Err("source migration was not initialized".into()),
        Command::Status => {}
        Command::Queue {
            block,
            prover,
            cycle_limit,
        } => {
            let blocks = block
                .iter()
                .map(|p| block_bytes(p))
                .collect::<Result<Vec<_>>>()?;
            enqueue(
                &mut bootstrap,
                blocks,
                &LocalProver::new(prover),
                cycle_limit,
            )?;
        }
        Command::Fill(options) => fill(&mut bootstrap, &options, limits)?,
        Command::Run(options) => super::bootstrap_run::run(&mut bootstrap, options, limits)?,
        Command::Export {
            job,
            out,
            prover,
            cycle_limit,
        } => {
            let job = match job {
                Job::Genesis => bootstrap.genesis_job()?,
                Job::Range { batch } => bootstrap.range_job(batch)?,
                Job::Continue { batch } => bootstrap.continue_job(batch)?,
                Job::Aggregate { batch } => bootstrap.aggregate_job(&batch)?,
                Job::Fold { batch } => bootstrap.fold_job(batch)?,
            };
            export(&job, &out, &prover, cycle_limit)?;
            println!("{}", serde_json::json!({"job":out,"proof_generated":false}));
        }
        Command::Accept { job, proof } => {
            let artifact = ProofArtifact::read(&proof)?;
            match job {
                Acceptance::Prefix => bootstrap.accept_prefix(&artifact)?,
                Acceptance::Range { batch } => bootstrap.accept_range(batch, &artifact)?,
                Acceptance::Aggregate { batch } => bootstrap.accept_aggregate(&batch, &artifact)?,
            }
        }
    }
    status(&bootstrap)
}
fn enqueue(
    bootstrap: &mut SourceBootstrap,
    blocks: Vec<Vec<u8>>,
    executor: &LocalProver,
    ceiling: u64,
) -> Result<()> {
    let metrics = executor.execute_source(&bootstrap.preview(blocks.clone())?)?;
    if metrics.cycles > ceiling {
        return Err("source range exceeds the configured proving cycle ceiling".into());
    }
    let batch = bootstrap.advance(blocks)?;
    eprintln!(
        "{}",
        serde_json::json!({"stage":"queued","batch":batch,"cycles":metrics.cycles})
    );
    Ok(())
}
fn export(job: &SourceJob, out: &Path, prover: &Path, ceiling: u64) -> Result<()> {
    if out.exists() {
        let existing = SourceJob::read_public(out)?;
        if !existing.matches(job) {
            return Err("existing job differs from the queued source work".into());
        }
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(out.join("job.json"))?)?;
        if manifest["cycles"]
            .as_u64()
            .is_none_or(|c| c == 0 || c > ceiling)
        {
            return Err("existing job exceeds the configured cycle ceiling".into());
        }
        return Ok(());
    }
    let metrics = LocalProver::new(prover).execute_source(job)?;
    if !metrics.journal_matches || metrics.cycles == 0 || metrics.cycles > ceiling {
        return Err("source execution exceeds the configured proving budget".into());
    }
    job.export_public(out, metrics.cycles)?;
    Ok(())
}
pub(super) fn status(bootstrap: &SourceBootstrap) -> Result<()> {
    let store = bootstrap.store();
    let height = store.candidate().height()?;
    let proved = store.evidence().map(|_| store.base().height);
    println!(
        "{}",
        serde_json::json!({"native_height":height,"proved_height":proved,"ranges":store.batches().iter().map(|b|serde_json::json!({"batch":b.id,"start":b.claim.start.height,"end":b.claim.end.height,"proved":b.evidence.is_some()})).collect::<Vec<_>>(),"current_prefix_proved":proved==Some(height),"current_chain_admitted":false,"swap_executed":false})
    );
    Ok(())
}

pub(super) fn fill(
    bootstrap: &mut SourceBootstrap,
    options: &FillOptions,
    limits: RangeLimits,
) -> Result<()> {
    let mut rpc = SourceRpc::new(options.rpc.clone(), bootstrap.store().policy().network)?;
    if let Some(cookie) = &options.rpc_cookie {
        rpc = rpc.with_cookie(cookie)?;
    }
    let executor = LocalProver::new(&options.prover);
    for _ in 0..options.batches {
        if bootstrap.store().batches().len() >= limits.batches {
            break;
        }
        let height = bootstrap.store().candidate().height()?;
        if rpc.hash(height)? != bootstrap.store().candidate().tip()? {
            return Err(
                "source RPC reports a competing branch; fork reconciliation is required".into(),
            );
        }
        let tip = rpc.height()?;
        let through = options.through_height.unwrap_or(tip).min(tip);
        if height >= through {
            break;
        }
        let end = height
            .checked_add(u32::from(options.batch_size))
            .ok_or("source height overflow")?
            .min(through);
        let mut blocks = (height + 1..=end)
            .map(|h| rpc.block(h))
            .collect::<Result<Vec<_>>>()?;
        let metrics = super::preflight::fit(&mut blocks, options.cycle_limit, |blocks| {
            executor.execute_source(&bootstrap.preview(blocks.to_vec())?)
        })?;
        let batch = bootstrap.advance(blocks)?;
        eprintln!(
            "{}",
            serde_json::json!({"stage":"queued","batch":batch,"cycles":metrics.cycles})
        );
    }
    Ok(())
}
