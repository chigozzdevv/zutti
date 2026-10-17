use crate::{
    CandidateIndex, Error, IndexContext, IndexProof, IndexSnapshot, Result, block::ParsedBlock,
};
use crate::{
    bootstrap::GenesisBlock,
    chain_history::{CandidateHistory, CommitmentTreeRoots},
    difficulty::HeaderCandidate,
    trees::CandidateTrees,
};
use zcash_protocol::consensus::BranchId;
use zutti_protocol::Digest;

#[derive(Debug)]
pub struct CandidateChain {
    index: CandidateIndex,
    tip: Digest,
}
impl CandidateChain {
    pub fn new(context: IndexContext, anchor: Digest) -> Result<Self> {
        Digest::nonzero(anchor.0)?;
        Ok(Self {
            index: CandidateIndex::new(context),
            tip: anchor,
        })
    }
    pub fn append(&mut self, block: &ParsedBlock) -> Result<()> {
        if block.parent() != self.tip {
            return Err(Error::Parent);
        }
        self.index
            .append_block(block.claimed_height(), block.txids())?;
        self.tip = block.hash();
        Ok(())
    }
    pub fn snapshot(&self) -> IndexSnapshot {
        self.index.snapshot()
    }
    pub fn prove(&self, txid: Digest) -> IndexProof {
        self.index.prove(txid)
    }
    pub const fn tip(&self) -> Digest {
        self.tip
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TransitionError {
    #[error(transparent)]
    Source(#[from] Error),
    #[error(transparent)]
    History(#[from] crate::chain_history::Error),
    #[error(transparent)]
    Trees(#[from] crate::trees::Error),
    #[error(transparent)]
    Transaction(#[from] crate::transaction::Error),
    #[error(transparent)]
    Spends(#[from] crate::spends::Error),
    #[error(transparent)]
    Subsidy(#[from] crate::subsidy::Error),
    #[error("source prefix has not reached the configured index start")]
    IndexUnavailable,
    #[error("block does not commit to the computed Sapling tree root")]
    SaplingRoot,
}

#[derive(Debug)]
pub struct SourceCandidate {
    headers: HeaderCandidate,
    trees: CandidateTrees,
    history: CandidateHistory,
    index: CandidateIndex,
    ledger: crate::transaction::CandidateLedger,
    spends: crate::spends::CandidateSpends,
    monetary: crate::subsidy::MonetaryState,
}

impl SourceCandidate {
    pub fn from_genesis(
        genesis: &GenesisBlock,
        rules: Digest,
        index_start: u32,
    ) -> core::result::Result<Self, TransitionError> {
        let context = IndexContext::new(genesis.block().hash(), rules, index_start)?;
        Ok(Self {
            headers: HeaderCandidate::from_genesis(genesis)?,
            trees: CandidateTrees::default(),
            history: CandidateHistory::from_genesis(genesis),
            index: CandidateIndex::new(context),
            ledger: crate::transaction::CandidateLedger::default(),
            spends: crate::spends::CandidateSpends::default(),
            monetary: crate::subsidy::MonetaryState::new(genesis.network())?,
        })
    }
    pub fn append(&mut self, block: &ParsedBlock) -> core::result::Result<(), TransitionError> {
        let mut headers = self.headers.clone();
        headers.append(block)?;
        let mut ledger = self.ledger.clone();
        let value = ledger.append(block)?;
        let monetary = self.monetary.transition(block, value)?;
        let mut trees = self.trees.clone();
        trees.append(block.commitment_updates())?;
        let spends = self.spends.transition(block, &trees)?;
        let roots = trees.roots();
        if matches!(block.branch(), BranchId::Sapling | BranchId::Blossom)
            && roots.sapling.0 != block.header().final_sapling_root
        {
            return Err(TransitionError::SaplingRoot);
        }
        let mut history = self.history.clone();
        history.append(block, roots)?;
        if block.claimed_height() > self.index.snapshot().context.start() {
            self.index
                .append_block(block.claimed_height(), block.txids())?;
        }
        self.headers = headers;
        self.trees = trees;
        self.history = history;
        self.ledger = ledger;
        self.spends = spends;
        self.monetary = monetary;
        Ok(())
    }
    pub fn height(&self) -> Result<u32> {
        self.headers.height()
    }
    pub fn tip(&self) -> Result<Digest> {
        self.headers.tip()
    }
    pub fn cumulative_work(&self) -> Result<[u8; 32]> {
        self.headers.cumulative_work()
    }
    pub fn tree_roots(&self) -> CommitmentTreeRoots {
        self.trees.roots()
    }
    pub fn sprout_root(&self) -> Digest {
        self.trees.sprout_root()
    }
    pub fn ledger(&self) -> &crate::transaction::CandidateLedger {
        &self.ledger
    }
    pub fn spends(&self) -> &crate::spends::CandidateSpends {
        &self.spends
    }
    pub fn monetary(&self) -> &crate::subsidy::MonetaryState {
        &self.monetary
    }
    pub fn history_root(&self) -> core::result::Result<Option<Digest>, TransitionError> {
        Ok(self.history.root()?)
    }
    pub fn index_snapshot(&self) -> core::result::Result<IndexSnapshot, TransitionError> {
        let snapshot = self.index.snapshot();
        if self.headers.height()? < snapshot.context.start() {
            return Err(TransitionError::IndexUnavailable);
        }
        Ok(snapshot)
    }
    pub fn prove(&self, txid: Digest) -> core::result::Result<IndexProof, TransitionError> {
        self.index_snapshot()?;
        Ok(self.index.prove(txid))
    }
}
