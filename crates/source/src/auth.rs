use crate::{Error, Result, block::ParsedBlock};
use zcash_primitives::transaction::Transaction;
use zcash_protocol::consensus::BranchId;
use zutti_protocol::Digest;

pub(crate) fn transaction_auth_digest(tx: &Transaction) -> Digest {
    if tx.version().header() & 0x7fff_ffff < 5 {
        Digest([0xff; 32])
    } else {
        let mut digest = [0; 32];
        digest.copy_from_slice(tx.auth_commitment().as_bytes());
        Digest(digest)
    }
}

pub fn auth_data_root(leaves: &[Digest]) -> Result<Digest> {
    if leaves.is_empty() || leaves.len() > 100_000 {
        return Err(Error::Transactions);
    }
    let mut layer: Vec<_> = leaves.iter().map(|leaf| leaf.0).collect();
    layer.resize(layer.len().next_power_of_two(), [0; 32]);
    while layer.len() > 1 {
        layer = layer
            .chunks_exact(2)
            .map(|pair| personal_hash(b"ZcashAuthDatHash", &[&pair[0], &pair[1]]).0)
            .collect();
    }
    Ok(Digest(layer[0]))
}

pub fn block_commitment(previous_history_root: Digest, auth_data_root: Digest) -> Digest {
    personal_hash(
        b"ZcashBlockCommit",
        &[&previous_history_root.0, &auth_data_root.0, &[0; 32]],
    )
}

fn personal_hash(personalization: &[u8; 16], parts: &[&[u8]]) -> Digest {
    let mut state = blake2b_simd::Params::new()
        .hash_length(32)
        .personal(personalization)
        .to_state();
    for part in parts {
        state.update(part);
    }
    let mut digest = [0; 32];
    digest.copy_from_slice(state.finalize().as_bytes());
    Digest(digest)
}

pub fn check_authorizing_commitment(
    block: &ParsedBlock,
    previous_history_root: Digest,
) -> Result<()> {
    if !matches!(
        block.branch(),
        BranchId::Nu5 | BranchId::Nu6 | BranchId::Nu6_1 | BranchId::Nu6_2 | BranchId::Nu6_3
    ) || block_commitment(previous_history_root, block.auth_data_root()).0
        != block.header().final_sapling_root
    {
        return Err(Error::AuthorizingCommitment);
    }
    Ok(())
}
