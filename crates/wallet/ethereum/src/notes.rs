use crate::{
    ConsensusConfig, Error, FinalityEvidence, Result, mapping_key, verify_account,
    verify_finalized, verify_storage,
};
use alloy::{
    primitives::{Address, B256, U256},
    rpc::types::EIP1186AccountProofResponse,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoteRequest {
    pub chain_id: u64,
    pub pool: Address,
    pub pool_code_hash: B256,
    pub root: B256,
    pub nullifier: B256,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoteEvidence {
    pub consensus: FinalityEvidence,
    pub pool: EIP1186AccountProofResponse,
}

#[derive(Debug, Serialize)]
pub struct VerifiedNoteState {
    pub chain_id: u64,
    pub pool: Address,
    pub root: B256,
    pub nullifier: B256,
    pub execution_hash: B256,
    pub execution_height: u64,
    pub execution_timestamp: u64,
    pub finalized_slot: u64,
}

#[cfg(feature = "spv-fulfillment")]
#[derive(Debug, Serialize)]
pub struct VerifiedOwnedNote {
    pub commitment: String,
    pub amount_units: u128,
    pub state: VerifiedNoteState,
}

#[cfg(feature = "spv-fulfillment")]
#[derive(Debug)]
pub struct OwnedNoteRequest<'a> {
    pub domain: &'a zutti_protocol::domain::Domain,
    pub code_hash: B256,
    pub inventory: &'a zutti_wallet::inventory::PoolInventory,
    pub keys: &'a zutti_wallet::private_note::NoteKeys,
    pub position: u32,
}

#[cfg(feature = "spv-fulfillment")]
pub fn verify_owned(
    config: &ConsensusConfig,
    request: OwnedNoteRequest<'_>,
    evidence: &NoteEvidence,
    now: u64,
) -> Result<VerifiedOwnedNote> {
    let OwnedNoteRequest {
        domain,
        code_hash,
        inventory,
        keys,
        position,
    } = request;
    let owned = inventory
        .notes()
        .iter()
        .find(|owned| owned.path.position == position)
        .ok_or(Error::Storage)?;
    let id = domain.id().map_err(|_| Error::Configuration)?;
    if config.chain_id != domain.destination_chain()
        || owned.note.domain() != id
        || owned.note.recipient() != keys.recipient().map_err(|_| Error::Configuration)?
    {
        return Err(Error::Configuration);
    }
    let commitment = owned.note.commitment().map_err(|_| Error::Storage)?;
    owned
        .path
        .verify(id, commitment, inventory.root())
        .map_err(|_| Error::Storage)?;
    let request = NoteRequest {
        chain_id: config.chain_id,
        pool: Address::from(domain.pool()),
        pool_code_hash: code_hash,
        root: B256::from(inventory.root().0),
        nullifier: B256::from(
            keys.nullifier(&owned.note)
                .map_err(|_| Error::Configuration)?
                .0,
        ),
    };
    let state = verify(config, &request, evidence, now)?;
    Ok(VerifiedOwnedNote {
        commitment: commitment.to_string(),
        amount_units: owned.note.amount().units(),
        state,
    })
}

impl NoteRequest {
    pub fn storage_keys(&self) -> Result<[B256; 2]> {
        if self.chain_id == 0
            || self.pool.is_zero()
            || [self.pool_code_hash, self.root, self.nullifier]
                .iter()
                .any(B256::is_zero)
        {
            return Err(Error::Configuration);
        }
        Ok([mapping_key(self.root, 68), mapping_key(self.nullifier, 69)])
    }
}

pub fn verify(
    config: &ConsensusConfig,
    request: &NoteRequest,
    evidence: &NoteEvidence,
    now: u64,
) -> Result<VerifiedNoteState> {
    request.storage_keys()?;
    if config.chain_id != request.chain_id {
        return Err(Error::Configuration);
    }
    let execution = verify_finalized(config, &evidence.consensus, now)?;
    verify_note_storage(request, &evidence.pool, execution.header.state_root)?;
    Ok(VerifiedNoteState {
        chain_id: config.chain_id,
        pool: request.pool,
        root: request.root,
        nullifier: request.nullifier,
        execution_hash: execution.header.hash_slow(),
        execution_height: execution.header.number,
        execution_timestamp: execution.header.timestamp,
        finalized_slot: execution.finalized_slot,
    })
}

pub(crate) fn verify_note_storage(
    request: &NoteRequest,
    proof: &EIP1186AccountProofResponse,
    state_root: B256,
) -> Result<()> {
    let keys = request.storage_keys()?;
    verify_account(request.pool, request.pool_code_hash, proof, state_root)?;
    verify_storage(proof, &[(keys[0], U256::from(1)), (keys[1], U256::ZERO)])?;
    Ok(())
}
