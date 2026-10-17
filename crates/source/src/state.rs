use crate::{
    CandidateIndex, Error, IndexContext, IndexProof, IndexSnapshot, Result,
    access_index::HistoryIndex, block::ParsedBlock,
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
    State(#[from] crate::authenticated::StateError),
    #[error(transparent)]
    Encoding(#[from] zutti_protocol::Error),
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

#[derive(Debug, Clone)]
pub struct SourceCandidate {
    headers: HeaderCandidate,
    trees: CandidateTrees,
    history: CandidateHistory,
    index: HistoryIndex,
    ledger: crate::transaction::CandidateLedger,
    spends: crate::spends::CandidateSpends,
    monetary: crate::subsidy::MonetaryState,
    archive: crate::authenticated::AuthenticatedMap,
}

impl SourceCandidate {
    pub fn from_genesis(
        genesis: &GenesisBlock,
        rules: Digest,
        index_start: u32,
    ) -> core::result::Result<Self, TransitionError> {
        let context = IndexContext::new(genesis.block().hash(), rules, index_start)?;
        let mut candidate = Self {
            headers: HeaderCandidate::from_genesis(genesis)?,
            trees: CandidateTrees::default(),
            history: CandidateHistory::from_genesis(genesis),
            index: HistoryIndex::new(context),
            ledger: crate::transaction::CandidateLedger::default(),
            spends: crate::spends::CandidateSpends::default(),
            monetary: crate::subsidy::MonetaryState::new(genesis.network())?,
            archive: crate::authenticated::AuthenticatedMap::new(crate::checkpoint::context(
                genesis.block().hash(),
                rules,
            )),
        };
        candidate.record_header()?;
        Ok(candidate)
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
        let mut index = self.index.clone();
        if block.claimed_height() > index.snapshot().context.start() {
            index.append_block(block.claimed_height(), block.txids())?;
        }
        use zutti_protocol::encoding::Canonical;
        let mut archive = self.archive.clone();
        let record = crate::checkpoint::HeaderRecord {
            height: headers.height()?,
            hash: headers.tip()?,
            work: headers.cumulative_work()?,
            index: index.snapshot(),
        };
        if archive
            .set(
                crate::checkpoint::key(record.height),
                Some(record.to_bytes()?),
            )?
            .is_some()
        {
            return Err(Error::Height.into());
        }
        self.archive = archive;
        self.index = index;
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
        Ok(self.index.prove(txid)?)
    }
}

impl SourceCandidate {
    pub fn state_snapshot(
        &self,
    ) -> core::result::Result<crate::transition::StateSnapshot, TransitionError> {
        Ok(crate::transition::StateSnapshot {
            headers: self.headers.clone(),
            trees: self.trees.clone(),
            history: self.history.clone(),
            index: self.index.snapshot(),
            ledger_root: self.ledger.root(),
            ledger_count: self.ledger.count(),
            spends_root: self.spends.root(),
            nullifier_counts: self.spends.counts(),
            pool_balances: self.spends.balances(),
            monetary: self.monetary.clone(),
            archive_root: self.archive.root(),
        })
    }
    pub fn record_transition(
        &mut self,
        blocks: Vec<Vec<u8>>,
    ) -> core::result::Result<crate::transition::TransitionWitness, TransitionError> {
        use zutti_protocol::encoding::Canonical;
        if blocks.is_empty() || blocks.len() > crate::transition::MAX_BLOCKS {
            return Err(zutti_protocol::Error::Length.into());
        }
        let before = self.state_snapshot()?;
        let mut next = self.clone();
        next.ledger.start_recording()?;
        next.spends.start_recording()?;
        next.index.start_recording()?;
        next.archive.start_recording()?;
        for raw in &blocks {
            next.append_raw(raw)?;
        }
        let witness = crate::transition::TransitionWitness {
            before,
            ledger: next.ledger.take_transcript()?,
            spends: next.spends.take_transcript()?,
            index: next.index.take_transcript()?,
            archive: next.archive.take_transcript()?,
            blocks,
        };
        if witness.to_bytes()?.len() > crate::transition::MAX_WITNESS {
            return Err(zutti_protocol::Error::Length.into());
        }
        *self = next;
        Ok(witness)
    }
    pub(crate) fn append_raw(&mut self, raw: &[u8]) -> core::result::Result<(), TransitionError> {
        use zcash_protocol::consensus::{MAIN_NETWORK, NetworkType, TEST_NETWORK};
        let block = match self.headers.network() {
            NetworkType::Main => ParsedBlock::parse(raw, &MAIN_NETWORK)?,
            NetworkType::Test => ParsedBlock::parse(raw, &TEST_NETWORK)?,
            NetworkType::Regtest => return Err(Error::Network.into()),
        };
        self.append(&block)
    }
    pub(crate) fn from_transition(
        witness: &crate::transition::TransitionWitness,
    ) -> core::result::Result<Self, TransitionError> {
        let before = &witness.before;
        Ok(Self {
            headers: before.headers.clone(),
            trees: before.trees.clone(),
            history: before.history.clone(),
            index: HistoryIndex::witness(before.index, witness.index.clone()),
            ledger: crate::transaction::CandidateLedger::from_witness(
                before.ledger_root,
                before.ledger_count,
                witness.ledger.clone(),
            ),
            spends: crate::spends::CandidateSpends::from_witness(
                before.spends_root,
                before.nullifier_counts,
                before.pool_balances,
                witness.spends.clone(),
            ),
            monetary: before.monetary.clone(),
            archive: crate::authenticated::AuthenticatedMap::from_witness(
                crate::checkpoint::context(
                    before.index.context.genesis(),
                    before.index.context.rules(),
                ),
                before.archive_root,
                witness.archive.clone(),
            ),
        })
    }
    pub(crate) fn finish_witness(&self) -> core::result::Result<(), TransitionError> {
        self.ledger.finish_witness()?;
        self.spends.finish_witness()?;
        self.index.finish()?;
        self.archive.finish()?;
        Ok(())
    }
}

impl SourceCandidate {
    fn record_header(&mut self) -> core::result::Result<(), TransitionError> {
        use zutti_protocol::encoding::Canonical;
        let record = crate::checkpoint::HeaderRecord {
            height: self.headers.height()?,
            hash: self.headers.tip()?,
            work: self.headers.cumulative_work()?,
            index: self.index.snapshot(),
        };
        if self
            .archive
            .set(
                crate::checkpoint::key(record.height),
                Some(record.to_bytes()?),
            )?
            .is_some()
        {
            return Err(Error::Height.into());
        }
        Ok(())
    }
    pub fn prove_header(
        &self,
        height: u32,
    ) -> core::result::Result<crate::checkpoint::HeaderPath, TransitionError> {
        if height > self.height()? {
            return Err(Error::Height.into());
        }
        Ok(crate::checkpoint::HeaderPath {
            access: self.archive.prove(crate::checkpoint::key(height))?,
        })
    }
    pub fn checkpoint(
        &self,
        confirmations: u32,
    ) -> core::result::Result<crate::checkpoint::SourceCheckpoint, TransitionError> {
        if confirmations == 0 {
            return Err(Error::Height.into());
        }
        let height = self
            .height()?
            .checked_sub(confirmations)
            .ok_or(Error::Height)?;
        let context = self.index.snapshot().context;
        let stable = self.prove_header(height)?.verify(
            context.genesis(),
            context.rules(),
            self.archive.root(),
            height,
        )?;
        if height < context.start() {
            return Err(TransitionError::IndexUnavailable);
        }
        Ok(crate::checkpoint::SourceCheckpoint {
            stable,
            archive: self.archive.root(),
        })
    }
}
