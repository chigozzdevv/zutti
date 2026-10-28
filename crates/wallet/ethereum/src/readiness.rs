use crate::{
    ConsensusConfig, Error, FinalityEvidence, Result, SourceGuard, notes::NoteRequest,
    verify_finalized, verify_source_guard,
};
use alloy::{primitives::B256, rpc::types::EIP1186AccountProofResponse};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadinessRequest {
    pub note: NoteRequest,
    pub source: SourceGuard,
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
    if request.source.address == request.note.pool {
        return Err(Error::Configuration);
    }
    crate::notes::verify_note_storage(&request.note, pool, state_root)?;
    verify_source_guard(&request.source, source, state_root)
}
