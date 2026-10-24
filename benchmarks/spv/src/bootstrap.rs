use crate::{MAX_RECEIPT, Result, RunOptions, bounded, image, run, verify, write_new};
use bincode::Options;
use risc0_zkvm::Receipt;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{
    fs,
    fs::File,
    path::{Path, PathBuf},
};
use zcash_protocol::consensus::NetworkType;
use zutti_protocol::encoding::Canonical;
use zutti_source::spv::{
    HEADER_BYTES, HeaderClaim, MAX_HEADERS, MAX_WITNESS, Policy, Witness, parse_header,
};

const MAX_PROGRAM: usize = 32 * 1024 * 1024;
const MAX_JOURNAL: usize = 16_384;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    schema: u32,
    program_sha256: String,
    genesis_sha256: String,
    policy: String,
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Batch {
    schema: u32,
    index: u32,
    first_height: u32,
    last_height: u32,
    predecessor_sha256: Option<String>,
    witness_sha256: String,
    journal_sha256: String,
}

#[derive(Debug, Serialize)]
pub struct Status {
    pub policy_id: String,
    pub image_id: String,
    pub verified_batches: u32,
    pub verified_height: Option<u32>,
    pub verified_tip: Option<String>,
    pub pending_batch: Option<u32>,
    pub pending_height: Option<u32>,
    pub canonicality_admitted: bool,
    pub settlement_authority: bool,
}

struct Pending {
    directory: PathBuf,
    witness: Witness,
    journal: Vec<u8>,
}

pub struct Bootstrap {
    root: PathBuf,
    _lock: File,
    program: Vec<u8>,
    genesis: Vec<u8>,
    policy: Policy,
}

struct Snapshot {
    count: u32,
    previous: Option<HeaderClaim>,
    receipt: Option<Receipt>,
    pending: Option<Pending>,
}

fn sha(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn regular(path: &Path, limit: usize) -> Result<Vec<u8>> {
    if !fs::symlink_metadata(path)?.file_type().is_file() {
        return Err("bootstrap artifact must be a regular file, not a symlink".into());
    }
    bounded(path, limit)
}

fn directory(path: &Path) -> Result<()> {
    if !fs::symlink_metadata(path)?.file_type().is_dir() {
        return Err("bootstrap directory must not be a symlink".into());
    }
    Ok(())
}

fn sync(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

fn exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

pub fn headers(path: &Path) -> Result<Vec<Vec<u8>>> {
    let bytes = regular(path, HEADER_BYTES * MAX_HEADERS)?;
    if bytes.len() % HEADER_BYTES != 0 {
        return Err("header batch must contain consecutive, complete 1487-byte raw headers".into());
    }
    Ok(bytes
        .chunks_exact(HEADER_BYTES)
        .map(<[u8]>::to_vec)
        .collect())
}

impl Bootstrap {
    pub fn init(root: &Path, program: &[u8], genesis: &[u8], policy: Policy) -> Result<Self> {
        if program.is_empty() || program.len() > MAX_PROGRAM || image(program)? != policy.program()
        {
            return Err("bootstrap program does not match the pinned policy".into());
        }
        if policy.network() != NetworkType::Test {
            return Err("bootstrap runner currently supports Testnet only".into());
        }
        HeaderClaim::genesis(policy, genesis)?;
        if exists(root)? {
            return Err("bootstrap state already exists; open it to resume".into());
        }
        let parent = root
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let stage = tempfile::Builder::new()
            .prefix(".spv-init-")
            .tempdir_in(parent)?;
        write_new(&stage.path().join("lock"), b"zutti-spv-bootstrap-v1")?;
        write_new(&stage.path().join("program.elf"), program)?;
        write_new(&stage.path().join("genesis.bin"), genesis)?;
        fs::create_dir(stage.path().join("jobs"))?;
        write_new(
            &stage.path().join("config.json"),
            &serde_json::to_vec_pretty(&Config {
                schema: 1,
                program_sha256: sha(program),
                genesis_sha256: sha(genesis),
                policy: hex::encode(policy.to_bytes()?),
            })?,
        )?;
        sync(&stage.path().join("jobs"))?;
        sync(stage.path())?;
        fs::rename(stage.path(), root)?;
        sync(parent)?;
        Self::open(root)
    }

    pub fn open(root: &Path) -> Result<Self> {
        directory(root)?;
        regular(&root.join("lock"), 128)?;
        let lock = File::options()
            .read(true)
            .write(true)
            .open(root.join("lock"))?;
        lock.try_lock()
            .map_err(|_| "bootstrap state is already in use")?;
        directory(&root.join("jobs"))?;
        let config: Config = serde_json::from_slice(&regular(&root.join("config.json"), 4096)?)?;
        let program = regular(&root.join("program.elf"), MAX_PROGRAM)?;
        let genesis = regular(&root.join("genesis.bin"), HEADER_BYTES)?;
        let policy = Policy::from_bytes(&hex::decode(&config.policy)?)?;
        if config.schema != 1
            || sha(&program) != config.program_sha256
            || sha(&genesis) != config.genesis_sha256
            || image(&program)? != policy.program()
            || policy.network() != NetworkType::Test
        {
            return Err("bootstrap configuration or program identity changed".into());
        }
        HeaderClaim::genesis(policy, &genesis)?;
        Ok(Self {
            root: root.to_path_buf(),
            _lock: lock,
            program,
            genesis,
            policy,
        })
    }

    fn snapshot(&self) -> Result<Snapshot> {
        let jobs = self.root.join("jobs");
        let mut indices = Vec::new();
        for entry in fs::read_dir(&jobs)? {
            let entry = entry?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| "invalid bootstrap job name")?;
            if name.starts_with(".spv-prepare-") {
                directory(&entry.path())?;
                continue;
            }
            let index: u32 = name.parse()?;
            if name != format!("{index:08}") || index > self.policy.valid_through() {
                return Err("invalid bootstrap job index".into());
            }
            indices.push(index);
        }
        indices.sort_unstable();
        let mut snapshot = Snapshot {
            count: 0,
            previous: None,
            receipt: None,
            pending: None,
        };
        for (expected, index) in indices.into_iter().enumerate() {
            if index as usize != expected || snapshot.pending.is_some() {
                return Err("bootstrap jobs contain a gap or an unverified predecessor".into());
            }
            let job = jobs.join(format!("{index:08}"));
            directory(&job)?;
            let input = regular(&job.join("witness.bin"), MAX_WITNESS)?;
            let witness = Witness::from_bytes(&input)?;
            let journal = regular(&job.join("journal.bin"), MAX_JOURNAL)?;
            let claim = HeaderClaim::from_bytes(&journal)?;
            let manifest: Batch = serde_json::from_slice(&regular(&job.join("batch.json"), 4096)?)?;
            let (first_height, last_height) =
                self.shape(&witness, &claim, snapshot.previous.as_ref())?;
            let expected = Batch {
                schema: 1,
                index,
                first_height,
                last_height,
                predecessor_sha256: witness.required_journal()?.as_deref().map(sha),
                witness_sha256: sha(&input),
                journal_sha256: sha(&journal),
            };
            if manifest != expected {
                return Err("bootstrap batch identity changed".into());
            }
            let receipt_path = job.join("receipt.bin");
            if exists(&receipt_path)? {
                let receipt: Receipt =
                    crate::codec().deserialize(&regular(&receipt_path, MAX_RECEIPT)?)?;
                verify(&receipt, self.policy.program(), &journal)?;
                snapshot.previous = Some(claim);
                snapshot.receipt = Some(receipt);
                snapshot.count += 1;
            } else {
                snapshot.pending = Some(Pending {
                    directory: job,
                    witness,
                    journal,
                });
            }
        }
        Ok(snapshot)
    }

    fn shape(
        &self,
        witness: &Witness,
        claim: &HeaderClaim,
        previous: Option<&HeaderClaim>,
    ) -> Result<(u32, u32)> {
        if witness.policy() != self.policy || claim.policy() != self.policy {
            return Err("bootstrap batch policy mismatch".into());
        }
        let (first, headers) = match (witness, previous) {
            (
                Witness::Genesis {
                    policy,
                    header,
                    headers,
                },
                None,
            ) if *policy == self.policy && *header == self.genesis => (0, headers),
            (
                Witness::Headers {
                    previous: input,
                    headers,
                },
                Some(previous),
            ) if input.to_bytes()? == previous.to_bytes()? && !headers.is_empty() => {
                (previous.height()? + 1, headers)
            }
            _ => {
                return Err(
                    "bootstrap batch is not rooted in its exact verified predecessor".into(),
                );
            }
        };
        if headers.len() > MAX_HEADERS {
            return Err("bootstrap header batch exceeds its bound".into());
        }
        let last = if previous.is_none() {
            headers.len() as u32
        } else {
            first + headers.len() as u32 - 1
        };
        let tip = zutti_protocol::Digest(
            parse_header(headers.last().unwrap_or(&self.genesis))?
                .hash()
                .0,
        );
        if claim.height()? != last || claim.tip()? != tip || last > self.policy.valid_through() {
            return Err("bootstrap batch output does not match its header range".into());
        }
        Ok((first, last))
    }

    pub fn status(&self) -> Result<Status> {
        let snapshot = self.snapshot()?;
        Ok(Status {
            policy_id: self.policy.id()?.to_string(),
            image_id: self.policy.program().to_string(),
            verified_batches: snapshot.count,
            verified_height: snapshot
                .previous
                .as_ref()
                .map(HeaderClaim::height)
                .transpose()?,
            verified_tip: snapshot
                .previous
                .as_ref()
                .map(|c| c.tip().map(|v| v.to_string()))
                .transpose()?,
            pending_batch: snapshot.pending.as_ref().map(|_| snapshot.count),
            pending_height: snapshot
                .pending
                .as_ref()
                .map(|p| -> Result<u32> { Ok(HeaderClaim::from_bytes(&p.journal)?.height()?) })
                .transpose()?,
            canonicality_admitted: false,
            settlement_authority: false,
        })
    }

    pub fn prepare(&self, headers: Vec<Vec<u8>>) -> Result<PathBuf> {
        let snapshot = self.snapshot()?;
        let witness = match &snapshot.previous {
            Some(previous) => Witness::Headers {
                previous: previous.clone(),
                headers,
            },
            None => Witness::Genesis {
                policy: self.policy,
                header: self.genesis.clone(),
                headers,
            },
        };
        let input = witness.to_bytes()?;
        if input.len() > MAX_WITNESS {
            return Err("bootstrap witness exceeds its bound".into());
        }
        if let Some(pending) = snapshot.pending {
            if pending.witness.to_bytes()? == input {
                return Ok(pending.directory);
            }
            return Err("finish the pending batch before preparing another".into());
        }
        let journal = witness.evaluate()?.journal()?;
        let claim = HeaderClaim::from_bytes(&journal)?;
        let (first_height, last_height) =
            self.shape(&witness, &claim, snapshot.previous.as_ref())?;
        let jobs = self.root.join("jobs");
        let target = jobs.join(format!("{:08}", snapshot.count));
        let stage = tempfile::Builder::new()
            .prefix(".spv-prepare-")
            .tempdir_in(&jobs)?;
        write_new(&stage.path().join("witness.bin"), &input)?;
        write_new(&stage.path().join("journal.bin"), &journal)?;
        write_new(
            &stage.path().join("batch.json"),
            &serde_json::to_vec_pretty(&Batch {
                schema: 1,
                index: snapshot.count,
                first_height,
                last_height,
                predecessor_sha256: witness.required_journal()?.as_deref().map(sha),
                witness_sha256: sha(&input),
                journal_sha256: sha(&journal),
            })?,
        )?;
        sync(stage.path())?;
        fs::rename(stage.path(), &target)?;
        sync(&jobs)?;
        Ok(target)
    }

    pub fn accept(&self, batch: u32, path: &Path) -> Result<Status> {
        let snapshot = self.snapshot()?;
        let bytes = regular(path, MAX_RECEIPT)?;
        let receipt: Receipt = crate::codec().deserialize(&bytes)?;
        if batch < snapshot.count {
            let journal = regular(
                &self
                    .root
                    .join("jobs")
                    .join(format!("{batch:08}"))
                    .join("journal.bin"),
                MAX_JOURNAL,
            )?;
            verify(&receipt, self.policy.program(), &journal)?;
            return self.status();
        }
        if batch != snapshot.count {
            return Err("receipt is not for the next bootstrap batch".into());
        }
        let pending = snapshot.pending.ok_or("no prepared bootstrap batch")?;
        verify(&receipt, self.policy.program(), &pending.journal)?;
        let mut stage = tempfile::Builder::new()
            .prefix(".spv-receipt-")
            .tempfile_in(&pending.directory)?;
        use std::io::Write;
        stage.write_all(&bytes)?;
        stage.as_file().sync_all()?;
        stage.persist_noclobber(pending.directory.join("receipt.bin"))?;
        sync(&pending.directory)?;
        self.status()
    }

    pub fn prove(
        &self,
        out: &Path,
        cycle_limit: u64,
        public_header_receipt: bool,
    ) -> Result<Status> {
        let snapshot = self.snapshot()?;
        let pending = snapshot.pending.ok_or("no prepared bootstrap batch")?;
        run(
            &self.program,
            &pending.witness,
            snapshot.receipt.as_ref(),
            out,
            RunOptions {
                prove: true,
                cycle_limit,
                public_header_receipt,
            },
        )?
        .ok_or("prover did not return a genuine receipt")?;
        self.accept(snapshot.count, &out.join("receipt.bin"))
    }
}
