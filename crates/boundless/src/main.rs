use alloy::{
    primitives::U256,
    providers::{Provider, ProviderBuilder},
    signers::local::PrivateKeySigner,
};
use anyhow::{Context, Result, ensure};
use boundless_market::{
    Client, Deployment, StorageUploaderConfig,
    client::FundingMode,
    contracts::RequestStatus,
    price_oracle::{Amount, Asset},
    request_builder::OfferParams,
    storage::StorageUploaderType,
};
use clap::{Parser, Subcommand, ValueEnum};
use risc0_zkvm::{Journal, compute_image_id, sha::Digest};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use zeroize::{Zeroize, Zeroizing};

#[derive(Parser)]
#[command(name = "zutti-boundless")]
struct Args {
    #[arg(long, global = true, value_enum, default_value = "base")]
    network: Network,
    #[arg(long, default_value = ".data/boundless.env")]
    config: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum Network {
    Base,
    EthereumSepolia,
    BaseSepolia,
}
impl Network {
    const fn chain_id(self) -> u64 {
        match self {
            Self::Base => 8453,
            Self::EthereumSepolia => 11155111,
            Self::BaseSepolia => 84532,
        }
    }
    const fn fallback(self) -> &'static str {
        match self {
            Self::Base => "https://mainnet.base.org",
            Self::EthereumSepolia => "https://ethereum-sepolia-rpc.publicnode.com",
            Self::BaseSepolia => "https://sepolia.base.org",
        }
    }
    fn require_chain(self, actual: u64) -> Result<()> {
        ensure!(
            actual == self.chain_id(),
            "RPC chain does not match selected network"
        );
        Ok(())
    }
    fn require_deposit(self, amount: u64, cap: u64) -> Result<()> {
        ensure!(
            self != Self::Base,
            "deposits through this command are restricted to testnets"
        );
        ensure!(
            amount > 0 && cap > 0 && amount <= cap,
            "deposit exceeds configured testnet price cap"
        );
        Ok(())
    }
}

#[derive(Subcommand)]
enum Command {
    Balance,
    Deposit {
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
        amount_wei: u64,
    },
    Status {
        #[arg(long)]
        request: PathBuf,
    },
    Evidence {
        #[arg(long)]
        request: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    Submit {
        #[arg(long)]
        job: PathBuf,
        #[arg(long)]
        request: PathBuf,
    },
    Collect {
        #[arg(long)]
        request: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
}

struct Config(BTreeMap<String, String>);
impl Drop for Config {
    fn drop(&mut self) {
        for value in self.0.values_mut() {
            value.zeroize();
        }
    }
}
impl Config {
    fn read(path: &Path) -> Result<Self> {
        let bytes = private_read(path, 65_536)?;
        let text = std::str::from_utf8(&bytes).context("configuration encoding")?;
        let mut config = Self(BTreeMap::new());
        for item in dotenvy::from_read_iter(text.as_bytes()) {
            let (name, mut value) = item.map_err(|_| anyhow::anyhow!("invalid configuration"))?;
            if config.0.contains_key(&name) {
                value.zeroize();
                anyhow::bail!("duplicate configuration field");
            }
            config.0.insert(name, value);
        }
        Ok(config)
    }
    fn get(&self, name: &str) -> Result<&str> {
        self.0
            .get(name)
            .map(String::as_str)
            .with_context(|| format!("missing {name}"))
    }
    fn number(&self, name: &str) -> Result<u64> {
        self.get(name)?
            .parse()
            .with_context(|| format!("invalid {name}"))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Job {
    version: u16,
    visibility: String,
    image_id: String,
    program: PathBuf,
    stdin: PathBuf,
    journal: PathBuf,
    cycles: u64,
    #[serde(default)]
    program_sha256: Option<String>,
    #[serde(default)]
    stdin_sha256: Option<String>,
    #[serde(default)]
    journal_sha256: Option<String>,
}

impl Job {
    fn validate(&self) -> Result<()> {
        ensure!(
            matches!(self.version, 1 | 2) && self.visibility == "public",
            "this storage transport only accepts explicitly public witnesses"
        );
        ensure!(
            self.cycles > 0 && self.cycles <= 2_147_483_648,
            "unsupported cycle bound"
        );
        for hash in [
            &self.program_sha256,
            &self.stdin_sha256,
            &self.journal_sha256,
        ] {
            ensure!(
                hash.is_some() == (self.version == 2),
                "job hash schema mismatch"
            );
            if let Some(hash) = hash {
                ensure!(
                    hash.len() == 64 && hex::decode(hash)?.len() == 32,
                    "invalid artifact digest"
                );
            }
        }
        Ok(())
    }
    fn verify_artifacts(&self, program: &[u8], stdin: &[u8], journal: &[u8]) -> Result<()> {
        self.validate()?;
        for (expected, bytes) in [
            (&self.program_sha256, program),
            (&self.stdin_sha256, stdin),
            (&self.journal_sha256, journal),
        ] {
            if let Some(expected) = expected {
                ensure!(
                    *expected == hex::encode(Sha256::digest(bytes)),
                    "exported job artifact changed"
                );
            }
        }
        Ok(())
    }
}

fn proof_timeouts(cycles: u64) -> Result<(u32, u32)> {
    ensure!(
        cycles > 0 && cycles <= 2_147_483_648,
        "unsupported cycle bound"
    );
    let lock_timeout = u32::try_from(cycles.div_ceil(1_000_000) + 300)?.max(900);
    Ok((lock_timeout, (lock_timeout + 900).max(1800)))
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    version: u16,
    chain_id: u64,
    account: String,
    request_id: String,
    expires_at: u64,
    submitted_at: u64,
    image_id: String,
    journal: String,
    max_price_wei: String,
    input_visibility: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    submitted_block: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    proof_request: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct Proof {
    version: u16,
    chain_id: u64,
    request_id: String,
    image_id: String,
    journal: String,
    seal: String,
    input_visibility: String,
    elapsed_seconds: f64,
    request_to_receipt_seconds: u64,
}

fn private_read(path: &Path, maximum: usize) -> Result<Zeroizing<Vec<u8>>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let meta = file.metadata()?;
    ensure!(
        meta.is_file() && meta.permissions().mode() & 0o077 == 0,
        "input must be an owner-only regular file"
    );
    ensure!(
        meta.uid() == rustix::process::geteuid().as_raw(),
        "input owner mismatch"
    );
    ensure!(meta.len() <= maximum as u64, "input exceeds size limit");
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(maximum as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= maximum, "input exceeds size limit");
    Ok(bytes)
}

fn save_new(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n")?;
    file.as_file().sync_all()?;
    file.persist_noclobber(path).map_err(|e| e.error)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

fn now() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}

fn redact(message: &str, config: &Config) -> String {
    let mut message = Zeroizing::new(message.to_owned());
    for name in ["BOUNDLESS_PRIVATE_KEY", "PINATA_JWT", "BOUNDLESS_RPC_URL"] {
        if let Ok(value) = config.get(name)
            && !value.is_empty()
        {
            message = Zeroizing::new(message.replace(value, "[redacted]"));
            if name == "BOUNDLESS_PRIVATE_KEY" {
                let raw = value.strip_prefix("0x").unwrap_or(value);
                for representation in [
                    Zeroizing::new(raw.to_lowercase()),
                    Zeroizing::new(raw.to_uppercase()),
                ] {
                    message =
                        Zeroizing::new(message.replace(representation.as_str(), "[redacted]"));
                }
            }
        }
    }
    message.to_string()
}

async fn run(args: Args, config: &Config) -> Result<()> {
    ensure!(
        !risc0_zkvm::ProverOpts::default().dev_mode(),
        "development mode is not allowed"
    );
    let network = args.network;
    let rpc_url: url::Url = config.get("BOUNDLESS_RPC_URL")?.parse()?;
    let primary = ProviderBuilder::new().connect_http(rpc_url.clone());
    network.require_chain(primary.get_chain_id().await?)?;
    let signer: PrivateKeySigner = config
        .get("BOUNDLESS_PRIVATE_KEY")?
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid proving signer"))?;
    ensure!(
        format!("{:#x}", signer.address())
            .eq_ignore_ascii_case(config.get("BOUNDLESS_REQUESTOR_ADDRESS")?),
        "proving signer does not match configured account"
    );
    let mut storage = StorageUploaderConfig::default();
    storage.storage_uploader = StorageUploaderType::Pinata;
    storage.pinata_jwt = Some(config.get("PINATA_JWT")?.to_owned());
    let client = Client::builder()
        .with_rpc_url(rpc_url)
        .with_rpc_urls(vec![network.fallback().parse()?])
        .with_deployment(Some(
            Deployment::from_chain_id(network.chain_id())
                .context("selected deployment unavailable")?,
        ))
        .with_uploader_config(&storage)
        .await?
        .config_storage_layer(|c| c.inline_input_max_bytes(0))
        .with_skip_preflight(true)
        .with_funding_mode(FundingMode::Never)
        .with_private_key(signer)
        .build()
        .await
        .context("Boundless client initialization failed")?;
    let chain_id = client.provider().get_chain_id().await?;
    network.require_chain(chain_id)?;
    let account = client.caller();
    match args.command {
        Command::Deposit { amount_wei } => {
            network.require_deposit(amount_wei, config.number("BOUNDLESS_MAX_PRICE_WEI")?)?;
            client
                .boundless_market
                .deposit(U256::from(amount_wei))
                .await?;
            let balance = client.boundless_market.balance_of(account).await?;
            println!(
                "{}",
                serde_json::json!({
                    "chain_id":chain_id,"account":format!("{account:#x}"),
                    "deposited_wei":amount_wei.to_string(),"market_wei":balance.to_string(),
                })
            );
        }
        Command::Balance => {
            let balance = client.boundless_market.balance_of(account).await?;
            let wallet = client.provider().get_balance(account).await?;
            let cap = U256::from(config.number("BOUNDLESS_MAX_PRICE_WEI")?);
            ensure!(cap > U256::ZERO, "proof price cap must be positive");
            println!(
                "{}",
                serde_json::json!({"chain_id": chain_id, "account": format!("{account:#x}"), "market_wei": balance.to_string(), "wallet_wei": wallet.to_string(), "max_price_wei": cap.to_string(), "jobs_at_ceiling_excluding_gas": (balance / cap).to_string()})
            );
        }
        Command::Status { request } => {
            let record: Request = serde_json::from_slice(&private_read(&request, 150_000)?)?;
            ensure!(
                record.version == 1
                    && record.chain_id == chain_id
                    && record
                        .account
                        .eq_ignore_ascii_case(&format!("{account:#x}")),
                "request profile mismatch"
            );
            let id: U256 = record.request_id.parse()?;
            let status = client
                .boundless_market
                .get_status(id, Some(record.expires_at))
                .await?;
            println!(
                "{}",
                serde_json::json!({"request_id": record.request_id, "status": format!("{status:?}"), "elapsed_seconds": now()?.saturating_sub(record.submitted_at)})
            );
        }
        Command::Evidence { request, out } => {
            let record: Request = serde_json::from_slice(&private_read(&request, 150_000)?)?;
            ensure!(
                record.version == 1
                    && record.chain_id == chain_id
                    && record
                        .account
                        .eq_ignore_ascii_case(&format!("{account:#x}")),
                "request profile mismatch"
            );
            let id: U256 = record.request_id.parse()?;
            let locked = client
                .boundless_market
                .query_request_locked_event(id, record.submitted_block, None)
                .await?;
            let fulfilled = client
                .boundless_market
                .query_request_fulfilled_event(id, record.submitted_block, None)
                .await?;
            let provider = client.provider();
            let locked_block = provider
                .get_block_by_number(locked.block_number.into())
                .await?
                .context("lock block unavailable")?;
            let fulfilled_block = provider
                .get_block_by_number(fulfilled.block_number.into())
                .await?
                .context("fulfillment block unavailable")?;
            let price = locked
                .event
                .request
                .offer
                .price_at(locked_block.header.timestamp)?;
            let evidence = serde_json::json!({
                "version": 1, "chain_id": chain_id, "request_id": record.request_id,
                "image_id": record.image_id, "input_visibility": record.input_visibility,
                "submitted_at": record.submitted_at, "locked_at": locked_block.header.timestamp,
                "fulfilled_at": fulfilled_block.header.timestamp,
                "submission_to_fulfillment_seconds": fulfilled_block.header.timestamp.saturating_sub(record.submitted_at),
                "lock_to_fulfillment_seconds": fulfilled_block.header.timestamp.saturating_sub(locked_block.header.timestamp),
                "price_wei": price.to_string(), "fulfillment_block": fulfilled.block_number,
                "fulfillment_transaction": format!("{:#x}", fulfilled.tx_hash)
            });
            save_new(&out, &evidence)?;
            println!("{}", evidence);
        }
        Command::Submit { job, request } => {
            ensure!(
                !request.try_exists()?,
                "request record already exists; collect the existing request"
            );
            let job: Job = serde_json::from_slice(&private_read(&job, 16_384)?)?;
            job.validate()?;
            let program = private_read(&job.program, 64 * 1024 * 1024)?;
            let stdin = private_read(&job.stdin, 64 * 1024 * 1024 + 1044)?;
            let journal = private_read(&job.journal, 65_664)?;
            job.verify_artifacts(&program, &stdin, &journal)?;
            let image = Digest::from(
                <[u8; 32]>::try_from(hex::decode(&job.image_id)?)
                    .map_err(|_| anyhow::anyhow!("invalid image identifier"))?,
            );
            ensure!(
                compute_image_id(&program)? == image,
                "program image mismatch"
            );
            let cap = U256::from(config.number("BOUNDLESS_MAX_PRICE_WEI")?);
            let minimum = cap / U256::from(5);
            let balance = client.boundless_market.balance_of(account).await?;
            ensure!(
                cap > U256::ZERO && balance >= cap,
                "insufficient deposited proving balance"
            );
            let (lock_timeout, timeout) = proof_timeouts(job.cycles)?;
            let offer: OfferParams = OfferParams::builder()
                .min_price(Amount::new(minimum, Asset::ETH))
                .max_price(Amount::new(cap, Asset::ETH))
                .lock_collateral(Amount::new(
                    U256::from(20_u64) * U256::from(10_u64).pow(U256::from(18)),
                    Asset::ZKC,
                ))
                .ramp_up_period(60_u32)
                .lock_timeout(lock_timeout)
                .timeout(timeout)
                .into();
            let parameters = client
                .new_request()
                .with_program(program.to_vec())
                .with_stdin(stdin.to_vec())
                .with_image_id(image)
                .with_cycles(job.cycles)
                .with_journal(Journal::new(journal.to_vec()))
                .with_offer(offer)
                .with_groth16_proof();
            let mut proof_request = client.build_request(parameters).await?;
            proof_request.offer.rampUpStart = now()?;
            ensure!(
                proof_request.offer.maxPrice <= cap,
                "request exceeds authorized price cap"
            );
            let record = Request {
                version: 1,
                chain_id,
                account: format!("{account:#x}"),
                request_id: format!("{:#x}", proof_request.id),
                expires_at: proof_request.expires_at(),
                submitted_at: now()?,
                image_id: image.to_string(),
                journal: hex::encode(&journal),
                max_price_wei: cap.to_string(),
                input_visibility: "public".into(),
                submitted_block: Some(client.provider().get_block_number().await?),
                proof_request: Some(serde_json::to_value(&proof_request)?),
            };
            save_new(&request, &record)?;
            let submitted = if network == Network::Base {
                client.submit_request(&proof_request).await
            } else {
                client.submit_request_onchain(&proof_request).await
            };
            let (id, _) = submitted
                .context("submission failed; inspect the retained request before retrying")?;
            ensure!(
                format!("{id:#x}") == record.request_id,
                "submitted request identifier mismatch"
            );
            println!("{}", serde_json::to_string(&record)?);
        }
        Command::Collect { request, out } => {
            ensure!(!out.try_exists()?, "proof output already exists");
            let record: Request = serde_json::from_slice(&private_read(&request, 150_000)?)?;
            ensure!(
                record.version == 1
                    && record.chain_id == chain_id
                    && record
                        .account
                        .eq_ignore_ascii_case(&format!("{account:#x}")),
                "request profile mismatch"
            );
            let started = Instant::now();
            let id: U256 = record.request_id.parse()?;
            loop {
                let status = client
                    .boundless_market
                    .get_status(id, Some(record.expires_at))
                    .await?;
                match status {
                    RequestStatus::Fulfilled => break,
                    RequestStatus::Expired => anyhow::bail!("request expired without fulfillment"),
                    _ => tokio::time::sleep(Duration::from_secs(15)).await,
                }
            }
            let fulfillment = client
                .boundless_market
                .get_request_fulfillment(id, record.submitted_block, None)
                .await?;
            let data = fulfillment.data()?;
            let image = data.image_id().context("missing proof image")?;
            let journal = data.journal().context("missing proof journal")?;
            ensure!(
                hex::encode(<[u8; 32]>::from(image)) == record.image_id
                    && hex::encode(journal) == record.journal,
                "fulfilled proof statement mismatch"
            );
            ensure!(fulfillment.seal.len() == 260, "expected raw Groth16 proof");
            let proof = Proof {
                version: 1,
                chain_id,
                request_id: record.request_id,
                image_id: record.image_id,
                journal: record.journal,
                seal: hex::encode(&fulfillment.seal),
                input_visibility: record.input_visibility,
                elapsed_seconds: started.elapsed().as_secs_f64(),
                request_to_receipt_seconds: now()?.saturating_sub(record.submitted_at),
            };
            save_new(&out, &proof)?;
            println!(
                "{}",
                serde_json::json!({"proof_received": true, "cryptographically_verified": false, "out": out})
            );
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let args = Args::parse();
    let config = match Config::read(&args.config) {
        Ok(config) => config,
        Err(_) => {
            eprintln!("Boundless operation failed; check the private configuration");
            return std::process::ExitCode::from(2);
        }
    };
    match run(args, &config).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            let message = redact(&format!("{error:#}"), &config);
            eprintln!("Boundless operation failed: {message}");
            std::process::ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_network_rejects_other_chains_and_mainnet_deposits() -> Result<()> {
        for network in [
            Network::Base,
            Network::EthereumSepolia,
            Network::BaseSepolia,
        ] {
            network.require_chain(network.chain_id())?;
            for chain in [8453, 11155111, 84532, 1] {
                ensure!(network.require_chain(chain).is_ok() == (chain == network.chain_id()));
            }
        }
        ensure!(Network::Base.require_deposit(1, 10).is_err());
        for network in [Network::EthereumSepolia, Network::BaseSepolia] {
            network.require_deposit(10, 10)?;
            for (amount, cap) in [(0, 10), (11, 10), (1, 0)] {
                ensure!(network.require_deposit(amount, cap).is_err());
            }
        }
        let args = Args::try_parse_from([
            "zutti-boundless",
            "--network",
            "ethereum-sepolia",
            "balance",
        ])?;
        ensure!(args.network == Network::EthereumSepolia);
        Ok(())
    }

    #[test]
    fn large_jobs_receive_a_bounded_proving_window() -> Result<()> {
        ensure!(proof_timeouts(260_000_000)? == (900, 1800));
        ensure!(proof_timeouts(1_314_365_753)? == (1615, 2515));
        ensure!(proof_timeouts(2_147_483_648)? == (2448, 3348));
        ensure!(proof_timeouts(0).is_err());
        ensure!(proof_timeouts(u64::MAX).is_err());
        Ok(())
    }

    #[test]
    fn transport_rejects_private_jobs_and_unbounded_work() -> Result<()> {
        let mut job: Job = serde_json::from_value(serde_json::json!({
            "version":1, "visibility":"public", "image_id":"00".repeat(32),
            "program":"program.elf", "stdin":"stdin.bin", "journal":"journal.bin",
            "cycles":1,
        }))?;
        job.validate()?;
        job.visibility = "private".into();
        ensure!(job.validate().is_err());
        job.visibility = "public".into();
        for cycles in [268_435_456, 268_435_457, 2_147_483_648] {
            job.cycles = cycles;
            job.validate()?;
        }
        for cycles in [0, 2_147_483_649, u64::MAX] {
            job.cycles = cycles;
            ensure!(job.validate().is_err());
        }
        Ok(())
    }

    #[test]
    fn hashed_jobs_reject_changed_artifacts_and_incomplete_hash_schemas() -> Result<()> {
        let mut job: Job = serde_json::from_value(serde_json::json!({
            "version":2,"visibility":"public","image_id":"00".repeat(32),
            "program":"program.elf","stdin":"stdin.bin","journal":"journal.bin","cycles":1,
            "program_sha256":hex::encode(Sha256::digest(b"program")),
            "stdin_sha256":hex::encode(Sha256::digest(b"stdin")),
            "journal_sha256":hex::encode(Sha256::digest(b"journal")),
        }))?;
        job.verify_artifacts(b"program", b"stdin", b"journal")?;
        for (program, stdin, journal) in [
            (
                b"changed".as_slice(),
                b"stdin".as_slice(),
                b"journal".as_slice(),
            ),
            (
                b"program".as_slice(),
                b"changed".as_slice(),
                b"journal".as_slice(),
            ),
            (
                b"program".as_slice(),
                b"stdin".as_slice(),
                b"changed".as_slice(),
            ),
        ] {
            ensure!(job.verify_artifacts(program, stdin, journal).is_err());
        }
        job.stdin_sha256 = None;
        ensure!(job.validate().is_err());
        job.version = 1;
        ensure!(job.validate().is_err());
        Ok(())
    }

    #[test]
    fn retained_records_are_private_bounded_and_never_overwritten() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("record.json");
        let value = serde_json::json!({"value":1});
        save_new(&path, &value)?;
        ensure!(private_read(&path, 1024).is_ok());
        ensure!(private_read(&path, 1).is_err());
        ensure!(save_new(&path, &serde_json::json!({"value":2})).is_err());
        ensure!(serde_json::from_slice::<serde_json::Value>(&private_read(&path, 1024)?)? == value);
        let symlink = directory.path().join("symlink.json");
        std::os::unix::fs::symlink(&path, &symlink)?;
        ensure!(private_read(&symlink, 1024).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))?;
        ensure!(private_read(&path, 1024).is_err());
        Ok(())
    }

    #[test]
    fn invalid_configuration_does_not_echo_credentials() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("config.env");
        std::fs::write(&path, "PINATA_JWT=demo-secret\nPINATA_JWT=duplicate\n")?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        let error = match Config::read(&path) {
            Ok(_) => anyhow::bail!("duplicate field accepted"),
            Err(error) => format!("{error:#}"),
        };
        ensure!(!error.contains("demo-secret") && !error.contains("duplicate\n"));
        let config = Config(BTreeMap::from([
            (
                "BOUNDLESS_PRIVATE_KEY".into(),
                format!("0x{}", "ab".repeat(32)),
            ),
            ("PINATA_JWT".into(), "demo-secret".into()),
            (
                "BOUNDLESS_RPC_URL".into(),
                "https://demo.invalid/private".into(),
            ),
        ]));
        let error = format!(
            "0x{} {} {} demo-secret https://demo.invalid/private",
            "ab".repeat(32),
            "ab".repeat(32),
            "AB".repeat(32)
        );
        let sanitized = redact(&error, &config);
        ensure!(
            !sanitized.contains("abab")
                && !sanitized.contains("ABAB")
                && !sanitized.contains("demo-secret")
                && !sanitized.contains("demo.invalid")
        );
        Ok(())
    }
}
