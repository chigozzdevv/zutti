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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReservationRequest {
    pub chain_id: u64,
    pub pool: Address,
    pub pool_code_hash: B256,
    pub escrow: B256,
    pub order_tag: B256,
    pub nullifier: B256,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinalityEvidence {
    pub bootstrap: Bootstrap<MainnetConsensusSpec>,
    pub updates: Vec<Update<MainnetConsensusSpec>>,
    pub finality: FinalityUpdate<MainnetConsensusSpec>,
    pub execution: Header,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub consensus: FinalityEvidence,
    pub account: EIP1186AccountProofResponse,
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
    check_execution(
        &finalized,
        &evidence.execution,
        config.max_finalized_age,
        now,
    )?;
    Ok(VerifiedExecution {
        chain_id: config.chain_id,
        header: evidence.execution.clone(),
        finalized_slot: finalized.beacon().slot,
    })
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
    if proof.address != expected.pool || proof.code_hash != expected.pool_code_hash {
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
    let expected_slots = [
        (mapping_key(expected.escrow, 70), U256::from(1)),
        (mapping_key(expected.order_tag, 71), U256::from(1)),
        (mapping_key(expected.nullifier, 69), U256::ZERO),
    ];
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

#[cfg(test)]
mod tests;
