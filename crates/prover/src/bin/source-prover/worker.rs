use super::sync::repair_paths;
use std::{
    fs::{self, OpenOptions},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use zutti_prover::{
    artifact::ProofArtifact, local::LocalProver, remote::import_groth16, source::SourceJob,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
#[derive(Debug)]
pub(super) struct Worker {
    pub jobs: PathBuf,
    pub boundless: PathBuf,
    pub config: PathBuf,
    pub prover: PathBuf,
    pub cycle_limit: u64,
    pub workers: usize,
    pub submissions: usize,
}
struct Work<'a> {
    job: &'a SourceJob,
    directory: PathBuf,
}
impl Worker {
    pub fn prove(&mut self, jobs: &[SourceJob]) -> Result<Vec<ProofArtifact>> {
        if self.workers == 0 || self.workers > 8 {
            return Err("invalid proving worker count".into());
        }
        fs::create_dir_all(&self.jobs)?;
        let work = jobs
            .iter()
            .map(|job| self.prepare(job))
            .collect::<Result<Vec<_>>>()?;
        let new_requests = work
            .iter()
            .filter(|w| {
                !w.directory.join("source.proof").exists()
                    && !w.directory.join("result.json").exists()
                    && !w.directory.join("request.json").exists()
            })
            .count();
        if new_requests > self.submissions {
            return Err(
                "source proving submission budget is exhausted; queued work is retained".into(),
            );
        }
        for item in &work {
            let directory = &item.directory;
            if directory.join("source.proof").exists() {
                read_artifact(item)?;
                continue;
            }
            if directory.join("result.json").exists() {
                continue;
            }
            let request = directory.join("request.json");
            if !request.exists() {
                self.invoke(
                    directory,
                    "submit",
                    &[
                        "--job".as_ref(),
                        directory.join("job.json").as_os_str(),
                        "--request".as_ref(),
                        request.as_os_str(),
                    ],
                )?;
                self.submissions -= 1;
            }
            check_request(&request, item.job)?;
        }
        let mut failures = Vec::new();
        let worker = &*self;
        for group in work.chunks(self.workers) {
            let collected = std::thread::scope(|scope| {
                let handles = group
                    .iter()
                    .map(|item| {
                        scope.spawn(move || {
                            if item.directory.join("source.proof").exists()
                                || item.directory.join("result.json").exists()
                            {
                                return Ok(());
                            }
                            worker
                                .invoke(
                                    &item.directory,
                                    "collect",
                                    &[
                                        "--request".as_ref(),
                                        item.directory.join("request.json").as_os_str(),
                                        "--out".as_ref(),
                                        item.directory.join("result.json").as_os_str(),
                                    ],
                                )
                                .map_err(|e| e.to_string())
                        })
                    })
                    .collect::<Vec<_>>();
                handles
                    .into_iter()
                    .map(|handle| {
                        handle
                            .join()
                            .unwrap_or_else(|_| Err("source proof collector panicked".to_owned()))
                    })
                    .collect::<Vec<_>>()
            });
            for (item, result) in group.iter().zip(collected) {
                if let Err(error) = result {
                    failures.push(error);
                    continue;
                }
                let artifact = if item.directory.join("source.proof").exists() {
                    read_artifact(item)
                } else {
                    import_groth16(&item.directory.join("result.json"), item.job.statement())
                        .and_then(|artifact| {
                            artifact.persist_new(&item.directory.join("source.proof"))?;
                            Ok(artifact)
                        })
                };
                match artifact {
                    Ok(_) => eprintln!(
                        "{}",
                        serde_json::json!({"stage":"proved","statement":item.job.statement().digest()?.to_string()})
                    ),
                    Err(error) => failures.push(error.to_string()),
                }
            }
        }
        if let Some(error) = failures.first() {
            return Err(format!(
                "{} source jobs failed; requests and completed receipts are retained: {error}",
                failures.len()
            )
            .into());
        }
        work.iter().map(read_artifact).collect()
    }
    fn prepare<'a>(&self, job: &'a SourceJob) -> Result<Work<'a>> {
        let directory = self.jobs.join(job.id()?.to_string());
        if !directory.exists() {
            let temporary = tempfile::Builder::new()
                .prefix(".source-job-")
                .tempdir_in(&self.jobs)?;
            let export = temporary.path().join("job");
            let metrics = LocalProver::new(&self.prover).execute_source(job)?;
            if metrics.cycles == 0 || metrics.cycles > self.cycle_limit {
                return Err("source job exceeds the configured proving cycle ceiling".into());
            }
            job.export_public(&export, metrics.cycles)?;
            zutti_wallet::backup::persist_new(
                &export.join("execution.json"),
                &serde_json::to_vec(&metrics)?,
            )?;
            fs::rename(&export, &directory)?;
            fs::File::open(&self.jobs)?.sync_all()?;
        }
        if !SourceJob::read_public(&directory)?.matches(job) {
            return Err("retained job differs from the queued source statement or witness".into());
        }
        repair_paths(&directory, job, self.cycle_limit)?;
        if directory.join("request.json").exists() {
            check_request(&directory.join("request.json"), job)?;
        }
        Ok(Work { job, directory })
    }
    fn invoke(&self, directory: &Path, operation: &str, args: &[&std::ffi::OsStr]) -> Result<()> {
        let log_path = directory.join(format!("{operation}.log"));
        if log_path.symlink_metadata().is_ok_and(|m| {
            !m.is_file() || m.file_type().is_symlink() || m.permissions().mode() & 0o077 != 0
        }) {
            return Err("source proving log is not a regular file".into());
        }
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(log_path)?;
        let status = Command::new(&self.boundless)
            .arg("--config")
            .arg(&self.config)
            .arg(operation)
            .args(args)
            .stdout(Stdio::from(log.try_clone()?))
            .stderr(Stdio::from(log))
            .status()?;
        if !status.success() {
            return Err("Boundless operation failed; retained requests require recovery without replacement".into());
        }
        Ok(())
    }
}
fn read_artifact(work: &Work<'_>) -> Result<ProofArtifact> {
    let artifact = ProofArtifact::read(&work.directory.join("source.proof"))?;
    artifact.verify(work.job.statement())?;
    zutti_prover::source::groth16_seal(&artifact, work.job.statement())?;
    Ok(artifact)
}
fn check_request(path: &Path, job: &SourceJob) -> Result<()> {
    let value: serde_json::Value =
        serde_json::from_slice(&zutti_wallet::backup::read_private(path, 1_048_576)?)?;
    if value["version"] != 1
        || value["input_visibility"] != "public"
        || value["image_id"].as_str() != Some(&job.statement().program().to_string())
        || value["journal"].as_str() != Some(&hex::encode(job.statement().payload()))
    {
        return Err("retained request differs from the queued source job".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use bincode::Options;
    use risc0_zkvm::{FakeReceipt, InnerReceipt, Receipt, ReceiptClaim};
    use zcash_protocol::consensus::NetworkType;
    use zutti_prover::{ProofEnvelope, source::SourcePrograms};
    use zutti_source::proof::{SourceInput, SourceWitness};
    fn job() -> SourceJob {
        let programs = SourcePrograms::current();
        SourceJob::transition_with(
            &SourceWitness {
                policy: programs.policy(NetworkType::Test, 0, 3),
                input: SourceInput::Genesis(zebra_test::vectors::TESTNET_BLOCKS[&0].to_vec()),
            },
            None,
            &programs,
        )
        .unwrap()
    }
    fn worker(directory: &Path) -> Worker {
        Worker {
            jobs: directory.to_path_buf(),
            boundless: directory.join("must-not-run"),
            config: directory.join("missing.env"),
            prover: directory.join("must-not-execute"),
            cycle_limit: 268_435_456,
            workers: 2,
            submissions: 0,
        }
    }
    fn export(worker: &Worker, job: &SourceJob) -> PathBuf {
        let path = worker.jobs.join(job.id().unwrap().to_string());
        job.export_public(&path, 1_000_000).unwrap();
        path
    }
    #[test]
    fn job_identity_binds_exact_inputs_even_when_the_claim_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let worker = worker(dir.path());
        let job = job();
        let path = export(&worker, &job);
        let restored = SourceJob::read_public(&path).unwrap();
        assert_eq!(restored.id().unwrap(), job.id().unwrap());
        let mut input = fs::read(path.join("stdin.bin")).unwrap();
        input.push(0);
        fs::write(path.join("stdin.bin"), input).unwrap();
        let changed = SourceJob::read_public(&path).unwrap();
        assert_eq!(changed.statement(), job.statement());
        assert_ne!(changed.id().unwrap(), job.id().unwrap());
        assert!(!changed.matches(&job));
    }
    #[test]
    fn submission_budget_is_checked_before_any_remote_operation() {
        let dir = tempfile::tempdir().unwrap();
        let mut worker = worker(dir.path());
        let job = job();
        let path = export(&worker, &job);
        let result = worker.prove(&[job]).unwrap_err().to_string();
        assert!(result.contains("budget is exhausted"));
        assert!(!path.join("request.json").exists());
    }
    #[test]
    fn altered_retained_requests_fail_without_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let mut worker = worker(dir.path());
        let job = job();
        let path = export(&worker, &job);
        let request = serde_json::json!({"version":1,"input_visibility":"public","image_id":job.statement().program().to_string(),"journal":"00"});
        let bytes = serde_json::to_vec(&request).unwrap();
        zutti_wallet::backup::persist_new(&path.join("request.json"), &bytes).unwrap();
        assert!(
            worker
                .prove(&[job])
                .unwrap_err()
                .to_string()
                .contains("request differs")
        );
        assert_eq!(fs::read(path.join("request.json")).unwrap(), bytes);
        assert!(!path.join("result.json").exists());
    }
    #[test]
    fn fake_cached_receipts_do_not_skip_verification() {
        let dir = tempfile::tempdir().unwrap();
        let mut worker = worker(dir.path());
        let job = job();
        let path = export(&worker, &job);
        let statement = job.statement();
        let receipt = Receipt::new(
            InnerReceipt::Fake(FakeReceipt::new(ReceiptClaim::ok(
                statement.program().0,
                statement.payload().to_vec(),
            ))),
            statement.payload().to_vec(),
        );
        let artifact = ProofArtifact::new(
            statement.clone(),
            ProofEnvelope::new(
                statement,
                bincode::DefaultOptions::new()
                    .with_fixint_encoding()
                    .serialize(&receipt)
                    .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        artifact.persist_new(&path.join("source.proof")).unwrap();
        assert!(worker.prove(&[job]).is_err());
        assert!(!path.join("request.json").exists());
    }
}
