use crate::{
    ConsensusConfig, Error, FinalityEvidence, Result, mapping_key, verify_account,
    verify_finalized, verify_storage,
};
use alloy::{
    primitives::{Address, B256, U256},
    rpc::types::EIP1186AccountProofResponse,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InventoryRequest {
    pub chain_id: u64,
    pub pool: Address,
    pub pool_code_hash: B256,
    pub source: Address,
    pub source_code_hash: B256,
    pub root: B256,
    pub checkpoint: B256,
    pub order_tag: B256,
    pub nullifiers: Vec<B256>,
    pub expiry: u32,
    pub confirmations: u32,
    pub minimum_remaining: u32,
    pub maximum_source_age: u64,
    pub future_drift: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InventoryEvidence {
    pub consensus: FinalityEvidence,
    pub pool: EIP1186AccountProofResponse,
    pub source: EIP1186AccountProofResponse,
}

#[derive(Debug, Serialize)]
pub struct VerifiedInventory {
    chain_id: u64,
    execution_hash: B256,
    execution_height: u64,
    execution_timestamp: u64,
    finalized_slot: u64,
}

pub fn verify(
    config: &ConsensusConfig,
    request: &InventoryRequest,
    evidence: &InventoryEvidence,
    now: u64,
) -> Result<VerifiedInventory> {
    check_request(request)?;
    if config.chain_id != request.chain_id {
        return Err(Error::Configuration);
    }
    let execution = verify_finalized(config, &evidence.consensus, now)?;
    verify_inventory_storage(
        request,
        &evidence.pool,
        &evidence.source,
        execution.header.state_root,
        now,
    )?;
    Ok(VerifiedInventory {
        chain_id: config.chain_id,
        execution_hash: execution.header.hash_slow(),
        execution_height: execution.header.number,
        execution_timestamp: execution.header.timestamp,
        finalized_slot: execution.finalized_slot,
    })
}

fn check_request(request: &InventoryRequest) -> Result<()> {
    if request.chain_id == 0
        || request.pool.is_zero()
        || request.source.is_zero()
        || request.pool == request.source
        || [
            request.pool_code_hash,
            request.source_code_hash,
            request.root,
            request.checkpoint,
            request.order_tag,
        ]
        .iter()
        .any(B256::is_zero)
        || request.nullifiers.is_empty()
        || request.nullifiers.len() > 8
        || request.nullifiers.iter().any(B256::is_zero)
        || request.nullifiers.iter().collect::<BTreeSet<_>>().len() != request.nullifiers.len()
        || request.expiry == 0
        || request.expiry >= 500_000_000
        || request.confirmations == 0
        || request.confirmations > 1000
        || request.minimum_remaining == 0
        || request.minimum_remaining > 128
        || request.maximum_source_age == 0
        || request.maximum_source_age > 86_400
        || request.future_drift > 7200
    {
        return Err(Error::Configuration);
    }
    Ok(())
}

fn slot(proof: &EIP1186AccountProofResponse, key: u64) -> Result<U256> {
    proof
        .storage_proof
        .iter()
        .find(|p| p.key.as_b256() == B256::from(U256::from(key).to_be_bytes::<32>()))
        .map(|p| p.value)
        .ok_or(Error::Storage)
}

fn verify_inventory_storage(
    request: &InventoryRequest,
    pool: &EIP1186AccountProofResponse,
    source: &EIP1186AccountProofResponse,
    state_root: B256,
    now: u64,
) -> Result<()> {
    check_request(request)?;
    verify_account(request.pool, request.pool_code_hash, pool, state_root)?;
    verify_account(request.source, request.source_code_hash, source, state_root)?;
    let mut pool_slots = vec![
        (mapping_key(request.root, 68), U256::from(1)),
        (mapping_key(request.order_tag, 71), U256::ZERO),
    ];
    pool_slots.extend(
        request
            .nullifiers
            .iter()
            .map(|key| (mapping_key(*key, 69), U256::ZERO)),
    );
    verify_storage(pool, &pool_slots)?;
    let state = slot(source, 2)?;
    let control = slot(source, 4)?;
    verify_storage(
        source,
        &[
            (B256::from(U256::from(2).to_be_bytes::<32>()), state),
            (B256::from(U256::from(4).to_be_bytes::<32>()), control),
            (mapping_key(request.checkpoint, 19), U256::from(1)),
        ],
    )?;
    if !(state >> 64_usize).is_zero() || !(control >> 64_usize).is_zero() {
        return Err(Error::Storage);
    }
    let height = (state & U256::from(u32::MAX)).to::<u32>();
    let timestamp = (state >> 32_usize).to::<u64>();
    if height == 0
        || timestamp > now.saturating_add(request.future_drift)
        || now > timestamp.saturating_add(request.maximum_source_age)
        || u64::from(height)
            + u64::from(request.confirmations)
            + u64::from(request.minimum_remaining)
            > u64::from(request.expiry)
    {
        return Err(Error::Storage);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
