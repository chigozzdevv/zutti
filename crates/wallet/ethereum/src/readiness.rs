use crate::{
    ConsensusConfig, Error, FinalityEvidence, Result, SourceGuard, mapping_key, notes::NoteRequest,
    verify_account, verify_finalized,
};
use alloy::{
    primitives::{B256, U256},
    rpc::types::EIP1186AccountProofResponse,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadinessRequest {
    pub note: NoteRequest,
    pub source: SourceGuard,
}

impl ReadinessRequest {
    pub fn source_keys(&self) -> Result<[B256; 3]> {
        if self.source.address.is_zero()
            || self.source.address == self.note.pool
            || self.source.code_hash.is_zero()
            || self.source.checkpoint.is_zero()
        {
            return Err(Error::Configuration);
        }
        Ok([
            B256::ZERO,
            B256::from(U256::from(4).to_be_bytes::<32>()),
            mapping_key(self.source.checkpoint, 19),
        ])
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadinessEvidence {
    pub consensus: FinalityEvidence,
    pub pool: EIP1186AccountProofResponse,
    pub source: EIP1186AccountProofResponse,
}

#[derive(Debug, Serialize)]
pub struct VerifiedReadiness {
    pub chain_id: u64,
    pub execution_hash: B256,
    pub execution_height: u64,
    pub execution_timestamp: u64,
    pub finalized_slot: u64,
    pub checkpoint: B256,
}

pub fn verify(
    config: &ConsensusConfig,
    request: &ReadinessRequest,
    evidence: &ReadinessEvidence,
    now: u64,
) -> Result<VerifiedReadiness> {
    request.note.storage_keys()?;
    if config.chain_id != request.note.chain_id || request.source.address == request.note.pool {
        return Err(Error::Configuration);
    }
    let execution = verify_finalized(config, &evidence.consensus, now)?;
    verify_storage(
        request,
        &evidence.pool,
        &evidence.source,
        execution.header.state_root,
    )?;
    Ok(VerifiedReadiness {
        chain_id: config.chain_id,
        execution_hash: execution.header.hash_slow(),
        execution_height: execution.header.number,
        execution_timestamp: execution.header.timestamp,
        finalized_slot: execution.finalized_slot,
        checkpoint: request.source.checkpoint,
    })
}

pub(crate) fn verify_storage(
    request: &ReadinessRequest,
    pool: &EIP1186AccountProofResponse,
    source: &EIP1186AccountProofResponse,
    state_root: B256,
) -> Result<()> {
    let keys = request.source_keys()?;
    crate::notes::verify_note_storage(&request.note, pool, state_root)?;
    verify_account(
        request.source.address,
        request.source.code_hash,
        source,
        state_root,
    )?;
    crate::verify_storage(
        source,
        &[
            (
                keys[0],
                U256::from_be_slice(request.source.checkpoint.as_slice()),
            ),
            (keys[1], U256::ZERO),
            (keys[2], U256::from(1)),
        ],
    )
}
