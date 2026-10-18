use super::{
    bootstrap::{FillOptions, fill, status},
    worker::Worker,
};
use clap::Args;
use std::path::PathBuf;
use zutti_prover::bootstrap::SourceBootstrap;
use zutti_source::storage::RangeLimits;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
#[derive(Debug, Args)]
pub(super) struct Options {
    #[command(flatten)]
    fill: FillOptions,
    #[arg(long)]
    jobs: PathBuf,
    #[arg(long)]
    boundless: PathBuf,
    #[arg(long, default_value = ".data/boundless.env")]
    boundless_config: PathBuf,
    #[arg(long, default_value_t = 4, value_parser = clap::value_parser!(u8).range(1..=8))]
    workers: u8,
    #[arg(long, default_value_t = 8, value_parser = clap::value_parser!(u32).range(0..=1024))]
    max_jobs: u32,
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..=64))]
    rounds: u32,
}
pub(super) fn run(
    bootstrap: &mut SourceBootstrap,
    options: Options,
    limits: RangeLimits,
) -> Result<()> {
    let mut worker = Worker {
        jobs: options.jobs,
        boundless: options.boundless,
        config: options.boundless_config,
        prover: options.fill.prover.clone(),
        cycle_limit: options.fill.cycle_limit,
        workers: usize::from(options.workers),
        submissions: options.max_jobs as usize,
    };
    for _ in 0..options.rounds {
        if bootstrap.store().batches().is_empty() {
            fill(bootstrap, &options.fill, limits)?;
        }
        let ids = bootstrap
            .store()
            .batches()
            .iter()
            .filter(|b| b.witness.is_some())
            .map(|b| b.id)
            .collect::<Vec<_>>();
        let genesis = bootstrap.store().evidence().is_none();
        let mut jobs = Vec::new();
        if genesis {
            jobs.push(bootstrap.genesis_job()?);
        }
        for id in &ids {
            jobs.push(bootstrap.range_job(*id)?);
        }
        let receipts = worker.prove(&jobs)?;
        let mut receipts = receipts.iter();
        if genesis {
            bootstrap.accept_prefix(receipts.next().ok_or("missing source genesis receipt")?)?;
        }
        for id in ids {
            bootstrap.accept_range(id, receipts.next().ok_or("missing source range receipt")?)?;
        }
        while bootstrap.store().batches().len() > 1 {
            let groups = bootstrap
                .store()
                .batches()
                .chunks(4)
                .filter(|g| g.len() > 1)
                .map(|group| group.iter().map(|b| b.id).collect::<Vec<_>>())
                .collect::<Vec<_>>();
            let jobs = groups
                .iter()
                .map(|ids| bootstrap.aggregate_job(ids))
                .collect::<Result<Vec<_>>>()?;
            let proofs = worker.prove(&jobs)?;
            for (ids, proof) in groups.iter().zip(proofs) {
                bootstrap.accept_aggregate(ids, &proof)?;
            }
        }
        if let Some(batch) = bootstrap.store().batches().first() {
            let jobs = [bootstrap.fold_job(batch.id)?];
            let proof = worker.prove(&jobs)?;
            bootstrap.accept_prefix(proof.first().ok_or("missing source prefix receipt")?)?;
        }
        status(bootstrap)?;
        if bootstrap.store().batches().is_empty() && jobs.is_empty() {
            break;
        }
    }
    Ok(())
}
