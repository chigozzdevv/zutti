use super::{Profile, print_store};
use clap::Args;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use zutti_prover::{
    artifact::ProofArtifact,
    local::LocalProver,
    pipeline::SourcePipeline,
    remote::import_groth16,
    rpc::SourceRpc,
    source::{MAX_SOURCE_CYCLES, SourceJob},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const DEFAULT_CYCLE_LIMIT: u64 = 268_435_456;

#[derive(Debug, Args)]
pub(super) struct Options {
    #[command(flatten)]
    profile: Profile,
    #[arg(long)]
    database: PathBuf,
    #[arg(long)]
    rpc: String,
    #[arg(long)]
    rpc_cookie: Option<PathBuf>,
    #[arg(long)]
    jobs: PathBuf,
    #[arg(long, default_value_t = 4, value_parser = clap::value_parser!(u8).range(1..=64))]
    batch_size: u8,
    #[arg(long, default_value_t = DEFAULT_CYCLE_LIMIT, value_parser = clap::value_parser!(u64).range(1..=MAX_SOURCE_CYCLES))]
    cycle_limit: u64,
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..))]
    batches: u32,
    #[arg(long)]
    through_height: Option<u32>,
    #[arg(long, default_value = "r0vm")]
    prover: PathBuf,
    #[arg(long)]
    boundless: PathBuf,
    #[arg(long, default_value = ".data/boundless.env")]
    boundless_config: PathBuf,
}

pub(super) fn run(options: Options) -> Result<()> {
    let programs = options.profile.programs()?;
    let policy = options.profile.policy(&programs);
    let mut rpc = SourceRpc::new(options.rpc.clone(), policy.network)?;
    if let Some(path) = &options.rpc_cookie {
        rpc = rpc.with_cookie(path)?;
    }
    let mut pipeline = if options.database.exists() {
        SourcePipeline::open(&options.database, policy, programs)?
    } else {
        SourcePipeline::create(&options.database, rpc.block(0)?, policy, programs)?
    };
    fs::create_dir_all(&options.jobs)?;
    let executor = LocalProver::new(&options.prover);
    for _ in 0..options.batches {
        if pipeline.store().pending().is_none() {
            let height = pipeline.store().candidate().height()?;
            if rpc.hash(height)? != pipeline.store().candidate().tip()? {
                return Err(
                    "source RPC reports a competing branch; fork reconciliation is required".into(),
                );
            }
            let tip = rpc.height()?;
            let through = options.through_height.unwrap_or(tip).min(tip);
            if height >= through {
                print_store(&pipeline)?;
                return Ok(());
            }
            let end = height
                .checked_add(u32::from(options.batch_size))
                .ok_or("source height overflow")?
                .min(through);
            let mut blocks = (height + 1..=end)
                .map(|h| rpc.block(h))
                .collect::<Result<Vec<_>>>()?;
            loop {
                let job = pipeline.preview(blocks.clone())?;
                let metrics = executor.execute_source(&job)?;
                eprintln!(
                    "{}",
                    serde_json::json!({"stage":"preflight","blocks":blocks.len(),"cycles":metrics.cycles,"seconds":metrics.seconds})
                );
                if metrics.cycles <= options.cycle_limit {
                    pipeline.advance(blocks)?;
                    break;
                }
                if blocks.len() == 1 {
                    return Err("a single validated source block exceeds the configured proving cycle ceiling".into());
                }
                blocks.truncate(blocks.len().div_ceil(2));
            }
        }
        let job = pipeline.job()?;
        let statement = job.statement().digest()?.to_string();
        let directory = options.jobs.join(statement);
        let artifact_path = directory.join("source.proof");
        if !directory.exists() {
            let temporary = tempfile::Builder::new()
                .prefix(".source-job-")
                .tempdir_in(&options.jobs)?;
            let export = temporary.path().join("job");
            let metrics = executor.execute_source(&job)?;
            if metrics.cycles > options.cycle_limit {
                return Err(
                    "pending source job exceeds the configured proving cycle ceiling".into(),
                );
            }
            job.export_public(&export, metrics.cycles)?;
            zutti_wallet::backup::persist_new(
                &export.join("execution.json"),
                &serde_json::to_vec(&metrics)?,
            )?;
            fs::rename(&export, &directory)?;
            repair_paths(&directory, &job, options.cycle_limit)?;
            fs::File::open(&options.jobs)?.sync_all()?;
        } else {
            let restored = SourceJob::read_public(&directory)?;
            if !restored.matches(&job) {
                return Err("pending job directory contains a different statement".into());
            }
            repair_paths(&directory, &job, options.cycle_limit)?;
        }
        if artifact_path.exists() {
            pipeline.accept(&ProofArtifact::read(&artifact_path)?)?;
        } else {
            let request = directory.join("request.json");
            let result = directory.join("result.json");
            if !request.exists() {
                invoke(
                    &options,
                    "submit",
                    &[
                        "--job",
                        text(&directory.join("job.json"))?,
                        "--request",
                        text(&request)?,
                    ],
                )?;
            }
            if !result.exists() {
                invoke(
                    &options,
                    "collect",
                    &["--request", text(&request)?, "--out", text(&result)?],
                )?;
            }
            let artifact = import_groth16(&result, job.statement())?;
            artifact.persist_new(&artifact_path)?;
            pipeline.accept(&artifact)?;
        }
        print_store(&pipeline)?;
    }
    Ok(())
}
fn text(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| "source job path is not UTF-8".into())
}
fn invoke(options: &Options, operation: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(&options.boundless)
        .arg("--config")
        .arg(&options.boundless_config)
        .arg(operation)
        .args(args)
        .stdout(std::process::Stdio::null())
        .status()?;
    if !status.success() {
        return Err(
            "Boundless operation failed; the pending batch and existing request are retained"
                .into(),
        );
    }
    Ok(())
}
pub(super) fn repair_paths(directory: &Path, job: &SourceJob, cycle_limit: u64) -> Result<()> {
    let path = directory.join("job.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&zutti_wallet::backup::read_private(&path, 16_384)?)?;
    if value["version"] != 1
        || value["visibility"] != "public"
        || value["image_id"].as_str() != Some(&job.statement().program().to_string())
        || !value["cycles"]
            .as_u64()
            .is_some_and(|c| c > 0 && c <= cycle_limit)
    {
        return Err("source job manifest differs from its expected program or limits".into());
    }
    let canonical = directory.canonicalize()?;
    for (key, name) in [
        ("program", "program.elf"),
        ("stdin", "stdin.bin"),
        ("journal", "journal.bin"),
    ] {
        value[key] = serde_json::to_value(canonical.join(name))?;
    }
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    use std::io::Write;
    temporary.write_all(&serde_json::to_vec_pretty(&value)?)?;
    temporary.as_file().sync_all()?;
    temporary.persist(&path).map_err(|e| e.error)?;
    fs::File::open(directory)?.sync_all()?;
    Ok(())
}
