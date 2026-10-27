#[cfg(feature = "fulfillment")]
pub mod authorization;
pub mod inventory;
pub mod notes;
pub mod transfers;
#[cfg(feature = "spv-fulfillment")]
pub mod spv_authorization;
#[cfg(feature = "spv-fulfillment")]
pub mod spv_funding;

use alloy::{
    consensus::{Header, TrieAccount},
    primitives::{Address, B256, U256, keccak256},
    rlp,
    rpc::types::EIP1186AccountProofResponse,
};
use alloy_trie::{Nibbles, proof::verify_proof};
use helios_consensus_core::{
    apply_bootstrap, apply_finality_update, apply_update,
    consensus_spec::{ConsensusSpec, MainnetConsensusSpec},
    types::{Bootstrap, FinalityUpdate, Forks, LightClientHeader, LightClientStore, Update},
    verify_bootstrap, verify_finality_update, verify_update,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid Ethereum verification configuration")]
    Configuration,
    #[error("invalid, stale or mismatched Ethereum consensus evidence")]
    Consensus,
    #[error("execution block does not match the finalized beacon header")]
    Execution,
    #[error("account proof or deployed pool code does not match the expected contract")]
    Account,
    #[error("storage proof does not prove the expected reserved escrow")]
    Storage,
    #[error("Ethereum evidence exceeds its bounds")]
    Bounds,
}
type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsensusConfig {
    pub chain_id: u64,
    pub genesis_time: u64,
    pub genesis_root: B256,
    pub forks: Forks,
    pub checkpoint: B256,
    pub seconds_per_slot: u64,
    pub max_checkpoint_age: u64,
    pub max_finalized_age: u64,
}
impl ConsensusConfig {
    pub fn sepolia(checkpoint: B256) -> Result<Self> {
        use alloy::primitives::{b256, fixed_bytes};
        use helios_consensus_core::types::Fork;
        if checkpoint.is_zero() {
            return Err(Error::Configuration);
        }
        Ok(Self {
            chain_id: 11155111,
            genesis_time: 1655733600,
            genesis_root: b256!("d8ea171f3c94aea21ebc42a1ed61052acf3f9209c00e4efbaaddac09ed9b8078"),
            checkpoint,
            seconds_per_slot: 12,
            max_checkpoint_age: 1_209_600,
            max_finalized_age: 1800,
            forks: Forks {
                genesis: Fork {
                    epoch: 0,
                    fork_version: fixed_bytes!("90000069"),
                },
                altair: Fork {
                    epoch: 50,
                    fork_version: fixed_bytes!("90000070"),
                },
                bellatrix: Fork {
                    epoch: 100,
                    fork_version: fixed_bytes!("90000071"),
                },
                capella: Fork {
                    epoch: 56832,
                    fork_version: fixed_bytes!("90000072"),
                },
                deneb: Fork {
                    epoch: 132608,
                    fork_version: fixed_bytes!("90000073"),
                },
                electra: Fork {
                    epoch: 222464,
                    fork_version: fixed_bytes!("90000074"),
                },
                fulu: Fork {
                    epoch: 272640,
                    fork_version: fixed_bytes!("90000075"),
                },
                gloas: Fork {
                    epoch: 353024,
                    fork_version: fixed_bytes!("90000076"),
                },
            },
        })
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReservationRequest {
    pub chain_id: u64,
    pub pool: Address,
    pub pool_code_hash: B256,
    pub escrow: B256,
    pub order_tag: B256,
    pub nullifier: B256,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceGuard>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceGuard {
    pub address: Address,
    pub code_hash: B256,
    pub checkpoint: B256,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinalityEvidence {
    pub bootstrap: Bootstrap<MainnetConsensusSpec>,
    pub updates: Vec<Update<MainnetConsensusSpec>>,
    pub finality: FinalityUpdate<MainnetConsensusSpec>,
    pub execution: Header,
    #[serde(default)]
    pub ancestry: Vec<Header>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub consensus: FinalityEvidence,
    pub account: EIP1186AccountProofResponse,
    #[serde(default)]
    pub source: Option<EIP1186AccountProofResponse>,
}
#[derive(Debug, Serialize)]
pub struct VerifiedExecution {
    chain_id: u64,
    header: Header,
    finalized_slot: u64,
}
#[derive(Debug, Serialize)]
pub struct VerifiedReservation {
    chain_id: u64,
    pool: Address,
    escrow: B256,
    execution_hash: B256,
    execution_height: u64,
    finalized_slot: u64,
    execution_timestamp: u64,
}

pub fn verify(
    config: &ConsensusConfig,
    expected: &ReservationRequest,
    evidence: &Evidence,
    now: u64,
) -> Result<VerifiedReservation> {
    if config.chain_id != expected.chain_id
        || expected.pool.is_zero()
        || expected.pool_code_hash.is_zero()
        || [expected.escrow, expected.order_tag, expected.nullifier]
            .iter()
            .any(B256::is_zero)
    {
        return Err(Error::Configuration);
    }
    let finalized = verify_finalized(config, &evidence.consensus, now)?;
    let execution = &finalized.header;
    verify_reserved_storage(expected, &evidence.account, execution.state_root)?;
    match (&expected.source, &evidence.source) {
        (Some(guard), Some(proof)) => verify_source_guard(guard, proof, execution.state_root)?,
        (None, None) => {}
        _ => return Err(Error::Storage),
    }
    Ok(VerifiedReservation {
        chain_id: config.chain_id,
        pool: expected.pool,
        escrow: expected.escrow,
        execution_hash: execution.hash_slow(),
        execution_height: execution.number,
        finalized_slot: finalized.finalized_slot,
        execution_timestamp: execution.timestamp,
    })
}

fn verify_source_guard(
    guard: &SourceGuard,
    proof: &EIP1186AccountProofResponse,
    root: B256,
) -> Result<()> {
    if guard.address.is_zero() || guard.code_hash.is_zero() || guard.checkpoint.is_zero() {
        return Err(Error::Configuration);
    }
    verify_account(guard.address, guard.code_hash, proof, root)?;
    let key = B256::from(U256::from(4).to_be_bytes::<32>());
    let control = proof
        .storage_proof
        .iter()
        .find(|value| value.key.as_b256() == key)
        .ok_or(Error::Storage)?
        .value;
    if !(control >> 64_usize).is_zero() {
        return Err(Error::Storage);
    }
    verify_storage(
        proof,
        &[
            (key, control),
            (mapping_key(guard.checkpoint, 19), U256::from(1)),
        ],
    )
}

pub fn verify_finalized(
    config: &ConsensusConfig,
    evidence: &FinalityEvidence,
    now: u64,
) -> Result<VerifiedExecution> {
    if config.chain_id == 0
        || config.genesis_root.is_zero()
        || config.checkpoint.is_zero()
        || config.seconds_per_slot == 0
        || config.seconds_per_slot > 60
        || config.max_checkpoint_age == 0
        || config.max_checkpoint_age > 1_209_600
        || config.max_finalized_age == 0
        || config.max_finalized_age > 86_400
    {
        return Err(Error::Configuration);
    }
    let finalized = verify_consensus(
        config,
        &evidence.bootstrap,
        &evidence.updates,
        &evidence.finality,
        now,
    )?;
    check_execution_path(
        &finalized,
        &evidence.execution,
        &evidence.ancestry,
        config.max_finalized_age,
        now,
    )?;
    Ok(VerifiedExecution {
        chain_id: config.chain_id,
        header: evidence.execution.clone(),
        finalized_slot: finalized.beacon().slot,
    })
}

fn check_execution_path(
    finalized: &LightClientHeader,
    execution: &Header,
    ancestry: &[Header],
    max_age: u64,
    now: u64,
) -> Result<()> {
    if ancestry.len() > 128 {
        return Err(Error::Bounds);
    }
    if execution.timestamp > now || now - execution.timestamp > max_age {
        return Err(Error::Execution);
    }
    let mut parent = execution;
    for child in ancestry {
        if child.parent_hash != parent.hash_slow()
            || parent.number.checked_add(1) != Some(child.number)
            || child.timestamp <= parent.timestamp
        {
            return Err(Error::Execution);
        }
        parent = child;
    }
    check_execution(finalized, parent, max_age, now)
}

fn check_execution(
    finalized: &LightClientHeader,
    execution: &Header,
    max_age: u64,
    now: u64,
) -> Result<()> {
    if execution.hash_slow() != execution_hash(finalized)?
        || execution.timestamp > now
        || now - execution.timestamp > max_age
    {
        return Err(Error::Execution);
    }
    Ok(())
}

fn verify_consensus<S: ConsensusSpec>(
    config: &ConsensusConfig,
    bootstrap: &Bootstrap<S>,
    updates: &[Update<S>],
    finality: &FinalityUpdate<S>,
    now: u64,
) -> Result<LightClientHeader> {
    if updates.len() > 64 {
        return Err(Error::Bounds);
    }
    let age = |slot: u64| -> Result<u64> {
        let timestamp = slot
            .checked_mul(config.seconds_per_slot)
            .and_then(|v| config.genesis_time.checked_add(v))
            .ok_or(Error::Consensus)?;
        now.checked_sub(timestamp).ok_or(Error::Consensus)
    };
    if age(bootstrap.header().beacon().slot)? > config.max_checkpoint_age {
        return Err(Error::Consensus);
    }
    let current_slot = now
        .checked_sub(config.genesis_time)
        .ok_or(Error::Consensus)?
        / config.seconds_per_slot;
    verify_bootstrap(bootstrap, config.checkpoint, &config.forks).map_err(|_| Error::Consensus)?;
    let mut store = LightClientStore::default();
    apply_bootstrap(&mut store, bootstrap);
    for update in updates {
        verify_update(
            update,
            current_slot,
            &store,
            config.genesis_root,
            &config.forks,
        )
        .map_err(|_| Error::Consensus)?;
        apply_update(&mut store, update);
    }
    verify_finality_update(
        finality,
        current_slot,
        &store,
        config.genesis_root,
        &config.forks,
    )
    .map_err(|_| Error::Consensus)?;
    apply_finality_update(&mut store, finality);
    let finalized = &store.finalized_header;
    if age(finalized.beacon().slot)? > config.max_finalized_age {
        return Err(Error::Consensus);
    }
    Ok(finalized.clone())
}

fn execution_hash(finalized: &LightClientHeader) -> Result<B256> {
    Ok(match finalized {
        LightClientHeader::Capella(value) => *value.execution.block_hash(),
        LightClientHeader::Deneb(value) => *value.execution.block_hash(),
        LightClientHeader::Electra(value) => *value.execution.block_hash(),
        LightClientHeader::Gloas(value) => value.execution_block_hash,
        LightClientHeader::Bellatrix(_) => return Err(Error::Execution),
    })
}

pub fn mapping_key(key: B256, slot: u64) -> B256 {
    let mut input = [0; 64];
    input[..32].copy_from_slice(key.as_slice());
    input[32..].copy_from_slice(&U256::from(slot).to_be_bytes::<32>());
    keccak256(input)
}
fn verify_reserved_storage(
    expected: &ReservationRequest,
    proof: &EIP1186AccountProofResponse,
    state_root: B256,
) -> Result<()> {
    verify_account(expected.pool, expected.pool_code_hash, proof, state_root)?;
    verify_storage(
        proof,
        &[
            (mapping_key(expected.escrow, 70), U256::from(1)),
            (mapping_key(expected.order_tag, 71), U256::from(1)),
            (mapping_key(expected.nullifier, 69), U256::ZERO),
        ],
    )
}

fn verify_account(
    address: Address,
    code_hash: B256,
    proof: &EIP1186AccountProofResponse,
    state_root: B256,
) -> Result<()> {
    if proof.address != address || proof.code_hash != code_hash {
        return Err(Error::Account);
    }
    bound_nodes(&proof.account_proof)?;
    let account = TrieAccount {
        nonce: proof.nonce,
        balance: proof.balance,
        storage_root: proof.storage_hash,
        code_hash: proof.code_hash,
    };
    verify_proof(
        state_root,
        Nibbles::unpack(keccak256(proof.address)),
        Some(rlp::encode(account)),
        &proof.account_proof,
    )
    .map_err(|_| Error::Account)?;
    Ok(())
}

fn verify_storage(
    proof: &EIP1186AccountProofResponse,
    expected_slots: &[(B256, U256)],
) -> Result<()> {
    if proof.storage_proof.len() != expected_slots.len() {
        return Err(Error::Storage);
    }
    let mut seen = BTreeSet::new();
    for storage in &proof.storage_proof {
        let key = storage.key.as_b256();
        if !seen.insert(key) || !expected_slots.contains(&(key, storage.value)) {
            return Err(Error::Storage);
        }
        bound_nodes(&storage.proof)?;
        let encoded = if storage.value.is_zero() {
            None
        } else {
            Some(rlp::encode(storage.value))
        };
        verify_proof(
            proof.storage_hash,
            Nibbles::unpack(keccak256(key)),
            encoded,
            &storage.proof,
        )
        .map_err(|_| Error::Storage)?;
    }
    Ok(())
}
fn bound_nodes(nodes: &[alloy::primitives::Bytes]) -> Result<()> {
    if nodes.len() > 128
        || nodes
            .iter()
            .any(|node| node.is_empty() || node.len() > 4096)
    {
        return Err(Error::Bounds);
    }
    Ok(())
}

#[cfg(all(test, feature = "spv-fulfillment"))]
mod test_support;
#[cfg(test)]
mod tests;
