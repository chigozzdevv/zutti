use crate::{Error, Result};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use zcash_encoding::CompactSize;
use zcash_primitives::{block::BlockHeader, transaction::Transaction};
use zcash_protocol::consensus::{BlockHeight, BranchId, Parameters};
use zcash_script::{Opcode, opcode::PossiblyBad};
use zutti_protocol::Digest;

pub const MAX_BLOCK_BYTES: usize = 2_000_000;

#[derive(Debug)]
pub struct ParsedBlock {
    header: BlockHeader,
    hash: Digest,
    parent: Digest,
    claimed_height: u32,
    txids: Vec<Digest>,
    versions: Vec<u32>,
    auth_root: Digest,
    branch: BranchId,
}
impl ParsedBlock {
    pub fn parse<P: Parameters>(bytes: &[u8], parameters: &P) -> Result<Self> {
        if bytes.is_empty() || bytes.len() > MAX_BLOCK_BYTES {
            return Err(Error::Block);
        }
        // Bound attacker-controlled solution/count fields before the upstream parser allocates.
        let mut probe = bytes.get(140..).ok_or(Error::Block)?;
        let solution: usize = CompactSize::read_t(&mut probe).map_err(|_| Error::Block)?;
        if solution > 2048 {
            return Err(Error::Block);
        }
        probe = probe.get(solution..).ok_or(Error::Block)?;
        let count: usize = CompactSize::read_t(&mut probe).map_err(|_| Error::Block)?;
        if count == 0 || count > bytes.len() / 10 || count > 100_000 {
            return Err(Error::Block);
        }
        let mut remaining = bytes;
        let header = BlockHeader::read(&mut remaining).map_err(|_| Error::Block)?;
        let parsed_count: usize = CompactSize::read_t(&mut remaining).map_err(|_| Error::Block)?;
        if count != parsed_count {
            return Err(Error::Block);
        }
        let coinbase =
            Transaction::read(&mut remaining, BranchId::Sprout).map_err(|_| Error::Block)?;
        let transparent = coinbase
            .transparent_bundle()
            .filter(|bundle| bundle.is_coinbase())
            .ok_or(Error::Block)?;
        if coinbase.sprout_bundle().is_some() {
            return Err(Error::Block);
        }
        let script = transparent.vin.first().ok_or(Error::Block)?.script_sig();
        let claimed_height = if header.prev_block.0 == [0; 32] {
            if Digest(header.hash().0) != crate::bootstrap::genesis_hash(parameters.network_type())?
            {
                return Err(Error::Genesis);
            }
            BlockHeight::from_u32(0)
        } else {
            match script.0.parse().next() {
                Some(Ok(PossiblyBad::Good(Opcode::PushValue(height)))) => {
                    BlockHeight::try_from(height.to_num().map_err(|_| Error::Block)?)
                        .map_err(|_| Error::Block)?
                }
                _ => return Err(Error::Block),
            }
        };
        let branch = BranchId::for_height(parameters, claimed_height);
        if coinbase.version().header() & 0x7fff_ffff >= 5
            && coinbase.consensus_branch_id() != branch
        {
            return Err(Error::Block);
        }
        let mut txids = vec![Digest(*coinbase.txid().as_ref())];
        let mut versions = vec![coinbase.version().header() & 0x7fff_ffff];
        let mut auth_digests = vec![crate::auth::transaction_auth_digest(&coinbase)];
        // Stream transactions: an untrusted count cannot preallocate a huge Vec<Transaction>.
        for _ in 1..count {
            let tx = Transaction::read(&mut remaining, branch).map_err(|_| Error::Block)?;
            if tx
                .transparent_bundle()
                .is_some_and(|bundle| bundle.is_coinbase())
            {
                return Err(Error::Block);
            }
            if tx.version().header() & 0x7fff_ffff >= 5 && tx.consensus_branch_id() != branch {
                return Err(Error::Block);
            }
            txids.push(Digest(*tx.txid().as_ref()));
            versions.push(tx.version().header() & 0x7fff_ffff);
            auth_digests.push(crate::auth::transaction_auth_digest(&tx));
        }
        if !remaining.is_empty() {
            return Err(Error::Block);
        }
        // Duplicate leaves must not exploit Bitcoin-style Merkle padding ambiguity.
        if txids.iter().copied().collect::<BTreeSet<_>>().len() != txids.len() {
            return Err(Error::Block);
        }
        if merkle_root(&txids)? != header.merkle_root {
            return Err(Error::Block);
        }
        Ok(Self {
            hash: Digest(header.hash().0),
            parent: Digest(header.prev_block.0),
            header,
            claimed_height: claimed_height.into(),
            txids,
            versions,
            auth_root: crate::auth::auth_data_root(&auth_digests)?,
            branch,
        })
    }
    pub const fn hash(&self) -> Digest {
        self.hash
    }
    pub const fn header(&self) -> &BlockHeader {
        &self.header
    }
    pub const fn parent(&self) -> Digest {
        self.parent
    }
    pub const fn claimed_height(&self) -> u32 {
        self.claimed_height
    }
    pub fn txids(&self) -> &[Digest] {
        &self.txids
    }
    pub fn versions(&self) -> &[u32] {
        &self.versions
    }
    pub const fn auth_data_root(&self) -> Digest {
        self.auth_root
    }
    pub const fn branch(&self) -> BranchId {
        self.branch
    }
}
fn merkle_root(txids: &[Digest]) -> Result<[u8; 32]> {
    if txids.is_empty() {
        return Err(Error::Block);
    }
    let mut layer: Vec<[u8; 32]> = txids.iter().map(|id| id.0).collect();
    while layer.len() > 1 {
        let mut next = Vec::with_capacity(layer.len().div_ceil(2));
        for pair in layer.chunks(2) {
            let right = pair.get(1).unwrap_or(&pair[0]);
            let mut hash = Sha256::new();
            hash.update(pair[0]);
            hash.update(right);
            next.push(Sha256::digest(hash.finalize()).into());
        }
        layer = next;
    }
    Ok(layer[0])
}
