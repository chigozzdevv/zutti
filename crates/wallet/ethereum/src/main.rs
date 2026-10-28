use clap::{Parser, Subcommand, ValueEnum};
use std::{
    io::Read,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use zutti_ethereum::{
    ConsensusConfig, Evidence, FinalityEvidence, ReservationRequest, verify, verify_finalized,
};
#[cfg(feature = "spv-fulfillment")]
mod execution;

#[derive(Debug, Parser)]
struct Args {
    #[command(subcommand)]
    command: Command,
}
#[derive(Debug, Subcommand)]
enum Command {
    Ready {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        request: PathBuf,
        #[arg(long)]
        evidence: PathBuf,
    },
    Transfer {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        request: PathBuf,
        #[arg(long)]
        evidence: PathBuf,
    },
    #[cfg(feature = "spv-fulfillment")]
    Swap(execution::Args),
    #[cfg(feature = "spv-fulfillment")]
    OwnedNote {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        deployment: PathBuf,
        #[arg(long)]
        directory: PathBuf,
        #[arg(long)]
        inventory: PathBuf,
        #[arg(long)]
        evidence: PathBuf,
        #[arg(long)]
        position: u32,
    },
    Configuration {
        #[arg(long)]
        checkpoint: alloy::primitives::B256,
    },
    ProofKeys {
        #[arg(long, value_enum)]
        kind: EvidenceKind,
        #[arg(long)]
        request: PathBuf,
    },
    Note {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        request: PathBuf,
        #[arg(long)]
        evidence: PathBuf,
    },
    Inventory {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        request: PathBuf,
        #[arg(long)]
        evidence: PathBuf,
    },
    Finality {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        evidence: PathBuf,
    },
    Reservation {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        request: PathBuf,
        #[arg(long)]
        evidence: PathBuf,
    },
}
#[derive(Debug, Clone, ValueEnum)]
enum EvidenceKind {
    Ready,
    Note,
    Reservation,
    Inventory,
}

fn proof_keys(
    kind: EvidenceKind,
    request: &Path,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    use alloy::primitives::{B256, U256};
    use zutti_ethereum::mapping_key;
    Ok(match kind {
        EvidenceKind::Ready => {
            let value = read::<zutti_ethereum::readiness::ReadinessRequest>(request, 16_384)?;
            serde_json::json!({
                "pool":{"address":value.note.pool,"keys":value.note.storage_keys()?},
                "source":{"address":value.source.address,"keys":[B256::from(U256::from(4).to_be_bytes::<32>()),mapping_key(value.source.checkpoint,19)]}
            })
        }
        EvidenceKind::Note => {
            let value = read::<zutti_ethereum::notes::NoteRequest>(request, 16_384)?;
            serde_json::json!({"pool":{"address":value.pool,"keys":value.storage_keys()?}})
        }
        EvidenceKind::Reservation => {
            let value = read::<ReservationRequest>(request, 16_384)?;
            let mut keys = serde_json::json!({"account":{"address":value.pool,"keys":[mapping_key(value.escrow,70),mapping_key(value.order_tag,71),mapping_key(value.nullifier,69)]}});
            if let Some(source) = value.source {
                keys["source"] = serde_json::json!({"address":source.address,"keys":[B256::from(U256::from(4).to_be_bytes::<32>()),mapping_key(source.checkpoint,19)]});
            }
            keys
        }
        EvidenceKind::Inventory => {
            let value = read::<zutti_ethereum::inventory::InventoryRequest>(request, 16_384)?;
            let mut keys = vec![
                mapping_key(value.root, 68),
                mapping_key(value.order_tag, 71),
            ];
            keys.extend(value.nullifiers.iter().map(|key| mapping_key(*key, 69)));
            serde_json::json!({
                "pool":{"address":value.pool,"keys":keys},
                "source":{"address":value.source,"keys":[B256::from(U256::from(2).to_be_bytes::<32>()),B256::from(U256::from(4).to_be_bytes::<32>()),mapping_key(value.checkpoint,19)]}
            })
        }
    })
}
fn read<T: serde::de::DeserializeOwned>(
    path: &Path,
    limit: u64,
) -> Result<T, Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err("verification input exceeds its bound".into());
    }
    Ok(serde_json::from_slice(&bytes)?)
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    match Args::parse().command {
        Command::Ready {
            config,
            request,
            evidence,
        } => {
            println!(
                "{}",
                serde_json::to_string(&zutti_ethereum::readiness::verify(
                    &read::<ConsensusConfig>(&config, 65_536)?,
                    &read::<zutti_ethereum::readiness::ReadinessRequest>(&request, 16_384)?,
                    &read::<zutti_ethereum::readiness::ReadinessEvidence>(&evidence, 16_777_216)?,
                    now,
                )?)?
            );
        }
        #[cfg(feature = "spv-fulfillment")]
        Command::Swap(args) => execution::run(args, now)?,
        #[cfg(feature = "spv-fulfillment")]
        Command::OwnedNote {
            config,
            deployment,
            directory,
            inventory,
            evidence,
            position,
        } => {
            use zutti_protocol::{Digest, Secret32, domain::Domain, encoding::Canonical};
            use zutti_wallet::{account::WalletAccount, backup, inventory::PoolInventory};
            let profile: serde_json::Value = read(&deployment, 65_536)?;
            if profile["network"] != "testnet" || profile["chain_id"] != 11155111 {
                return Err("Testnet deployment required".into());
            }
            let bytes = alloy::hex::decode(
                profile["domain_encoding"]
                    .as_str()
                    .ok_or("missing domain encoding")?,
            )?;
            let domain = Domain::from_bytes(&bytes)?;
            if domain.id()?.to_string() != profile["domain"].as_str().ok_or("missing domain")?
                || domain.destination_chain() != 11155111
                || domain.source_genesis() != zutti_protocol::network::ZCASH_TESTNET_GENESIS
                || alloy::primitives::Address::from(domain.pool())
                    != profile["pool"]
                        .as_str()
                        .ok_or("missing pool")?
                        .parse::<alloy::primitives::Address>()?
            {
                return Err("deployment domain mismatch".into());
            }
            let account = WalletAccount::open(&directory)?;
            let raw = backup::read_private(&directory.join("account.key"), 32)?;
            let key = Secret32::from_bytes(raw.as_slice().try_into()?)?;
            let archive = backup::read_private(&inventory, 262_144)?;
            let inventory =
                PoolInventory::from_bytes(&backup::open(domain.id()?, &archive, &key)?)?;
            let code = profile["code_hashes"]["pool"]
                .as_str()
                .ok_or("missing pool code hash")?;
            let code = Digest::from_hex(code.trim_start_matches("0x"))?;
            let result = zutti_ethereum::notes::verify_owned(
                &read::<ConsensusConfig>(&config, 65_536)?,
                zutti_ethereum::notes::OwnedNoteRequest {
                    domain: &domain,
                    code_hash: alloy::primitives::B256::from(code.0),
                    inventory: &inventory,
                    keys: account.note_keys(),
                    position,
                },
                &read::<zutti_ethereum::notes::NoteEvidence>(&evidence, 16_777_216)?,
                now,
            )?;
            println!("{}", serde_json::to_string(&result)?);
        }
        Command::Configuration { checkpoint } => {
            println!(
                "{}",
                serde_json::to_string_pretty(&ConsensusConfig::sepolia(checkpoint)?)?
            );
        }
        Command::Transfer {
            config,
            request,
            evidence,
        } => {
            println!(
                "{}",
                serde_json::to_string(&zutti_ethereum::transfers::verify(
                    &read::<ConsensusConfig>(&config, 65_536)?,
                    &read::<zutti_ethereum::transfers::TransferRequest>(&request, 16_384)?,
                    &read::<zutti_ethereum::transfers::TransferEvidence>(&evidence, 16_777_216)?,
                    now,
                )?)?
            );
        }
        Command::ProofKeys { kind, request } => {
            println!("{}", proof_keys(kind, &request)?);
        }
        Command::Note {
            config,
            request,
            evidence,
        } => {
            println!(
                "{}",
                serde_json::to_string(&zutti_ethereum::notes::verify(
                    &read::<ConsensusConfig>(&config, 65_536)?,
                    &read::<zutti_ethereum::notes::NoteRequest>(&request, 16_384)?,
                    &read::<zutti_ethereum::notes::NoteEvidence>(&evidence, 16_777_216)?,
                    now,
                )?)?
            );
        }
        Command::Inventory {
            config,
            request,
            evidence,
        } => {
            use zutti_ethereum::inventory::{InventoryEvidence, InventoryRequest, verify};
            println!(
                "{}",
                serde_json::to_string(&verify(
                    &read::<ConsensusConfig>(&config, 65_536)?,
                    &read::<InventoryRequest>(&request, 16_384)?,
                    &read::<InventoryEvidence>(&evidence, 16_777_216)?,
                    now,
                )?)?
            );
        }
        Command::Finality { config, evidence } => {
            let config: ConsensusConfig = read(&config, 65_536)?;
            let evidence: FinalityEvidence = read(&evidence, 16_777_216)?;
            println!(
                "{}",
                serde_json::to_string(&verify_finalized(&config, &evidence, now)?)?
            );
        }
        Command::Reservation {
            config,
            request,
            evidence,
        } => {
            let config: ConsensusConfig = read(&config, 65_536)?;
            let request: ReservationRequest = read(&request, 16_384)?;
            let evidence: Evidence = read(&evidence, 16_777_216)?;
            println!(
                "{}",
                serde_json::to_string(&verify(&config, &request, &evidence, now)?)?
            );
        }
    }
    Ok(())
}
fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
