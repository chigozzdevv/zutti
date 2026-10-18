use crate::{
    IndexProof, IndexSnapshot,
    authenticated::Transcript,
    chain_history::CandidateHistory,
    difficulty::HeaderCandidate,
    state::{SourceCandidate, TransitionError},
    subsidy::MonetaryState,
    trees::CandidateTrees,
};
use zutti_protocol::{
    Digest, Error,
    commitment::hash,
    encoding::{Canonical, Reader, Writer},
};

pub const MAX_BLOCKS: usize = 64;
pub const MAX_WITNESS: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct StateSnapshot {
    pub(crate) headers: HeaderCandidate,
    pub(crate) trees: CandidateTrees,
    pub(crate) history: CandidateHistory,
    pub(crate) index: IndexSnapshot,
    pub(crate) ledger_root: Digest,
    pub(crate) ledger_count: u64,
    pub(crate) spends_root: Digest,
    pub(crate) nullifier_counts: [u64; 4],
    pub(crate) pool_balances: [u64; 4],
    pub(crate) monetary: MonetaryState,
    pub(crate) archive_root: Digest,
}
impl StateSnapshot {
    pub fn commitment(&self) -> zutti_protocol::Result<Digest> {
        Ok(hash(b"source-state", &[&self.to_bytes()?]))
    }
    pub fn height(&self) -> crate::Result<u32> {
        self.headers.height()
    }
    pub fn timestamp(&self) -> crate::Result<u32> {
        self.headers.timestamp()
    }
    pub fn tip(&self) -> crate::Result<Digest> {
        self.headers.tip()
    }
    pub const fn index(&self) -> IndexSnapshot {
        self.index
    }
    pub const fn archive(&self) -> Digest {
        self.archive_root
    }
    pub fn work(&self) -> crate::Result<[u8; 32]> {
        self.headers.cumulative_work()
    }
    fn validate(&self) -> zutti_protocol::Result<()> {
        let height = self
            .headers
            .height()
            .map_err(|_| Error::Invalid("source height"))?;
        let tip = self
            .headers
            .tip()
            .map_err(|_| Error::Invalid("source tip"))?;
        let genesis = crate::bootstrap::genesis_hash(self.headers.network())
            .map_err(|_| Error::Invalid("source network"))?;
        if self.history.height() != height
            || self.history.tip() != tip
            || self.history.network() != self.headers.network()
            || self.index.context.genesis() != genesis
            || self.index.height != height.max(self.index.context.start())
        {
            return Err(Error::Invalid("source state context"));
        }
        self.monetary
            .validate_height(self.headers.network(), height)
            .map_err(|_| Error::Invalid("monetary activation"))?;
        for balance in self.pool_balances {
            if balance > crate::transaction::MAX_MONEY {
                return Err(Error::Invalid("shielded balance"));
            }
        }
        Ok(())
    }
}
impl Canonical for StateSnapshot {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        self.validate()?;
        writer.fixed(b"ZUTTISTA");
        writer.u16(if self.monetary.nsm().is_some() { 2 } else { 1 });
        self.headers.encode(writer)?;
        self.trees.encode(writer)?;
        self.history.encode(writer)?;
        self.index.encode(writer)?;
        writer.fixed(&self.ledger_root.0);
        writer.u64(self.ledger_count);
        writer.fixed(&self.spends_root.0);
        for count in self.nullifier_counts {
            writer.u64(count);
        }
        for balance in self.pool_balances {
            writer.u64(balance);
        }
        self.monetary
            .encode_state(writer, self.monetary.nsm().is_some())?;
        writer.fixed(&self.archive_root.0);
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTISTA" {
            return Err(Error::Version);
        }
        let version = reader.u16()?;
        if !matches!(version, 1 | 2) {
            return Err(Error::Version);
        }
        let state = Self {
            headers: HeaderCandidate::decode(reader)?,
            trees: CandidateTrees::decode(reader)?,
            history: CandidateHistory::decode(reader)?,
            index: IndexSnapshot::decode(reader)?,
            ledger_root: Digest::nonzero(reader.fixed()?)?,
            ledger_count: reader.u64()?,
            spends_root: Digest::nonzero(reader.fixed()?)?,
            nullifier_counts: [reader.u64()?, reader.u64()?, reader.u64()?, reader.u64()?],
            pool_balances: [reader.u64()?, reader.u64()?, reader.u64()?, reader.u64()?],
            monetary: MonetaryState::decode_state(reader, version == 2)?,
            archive_root: Digest::nonzero(reader.fixed()?)?,
        };
        state.validate()?;
        Ok(state)
    }
}

#[derive(Debug, Clone)]
pub struct TransitionWitness {
    pub(crate) before: StateSnapshot,
    pub(crate) ledger: Transcript,
    pub(crate) spends: Transcript,
    pub(crate) index: Vec<IndexProof>,
    pub(crate) archive: Transcript,
    pub(crate) blocks: Vec<Vec<u8>>,
}
impl TransitionWitness {
    pub const fn before(&self) -> &StateSnapshot {
        &self.before
    }
    pub fn evaluate(&self) -> Result<StateSnapshot, TransitionError> {
        self.validate()?;
        let mut state = SourceCandidate::from_transition(self)?;
        for raw in &self.blocks {
            state.append_raw(raw)?;
        }
        state.finish_witness()?;
        state.state_snapshot()
    }
    fn validate(&self) -> zutti_protocol::Result<()> {
        if self.blocks.is_empty()
            || self.blocks.len() > MAX_BLOCKS
            || self
                .blocks
                .iter()
                .any(|b| b.is_empty() || b.len() > 2_000_000)
            || self.index.len() > 100_000 * MAX_BLOCKS
        {
            return Err(Error::Length);
        }
        Ok(())
    }
}
impl Canonical for TransitionWitness {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        self.validate()?;
        writer.fixed(b"ZUTTITRN");
        writer.u16(1);
        self.before.encode(writer)?;
        self.ledger.encode(writer)?;
        self.spends.encode(writer)?;
        self.archive.encode(writer)?;
        writer.u32(self.index.len() as u32);
        for proof in &self.index {
            proof.encode(writer)?;
        }
        writer.u8(self.blocks.len() as u8);
        for block in &self.blocks {
            writer.bytes(block)?;
        }
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTITRN" || reader.u16()? != 1 {
            return Err(Error::Version);
        }
        let before = StateSnapshot::decode(reader)?;
        let ledger = Transcript::decode(reader)?;
        let spends = Transcript::decode(reader)?;
        let archive = Transcript::decode(reader)?;
        let count = reader.u32()? as usize;
        if count > MAX_BLOCKS * 100_000 {
            return Err(Error::Length);
        }
        let mut index = Vec::new();
        for _ in 0..count {
            index.push(IndexProof::decode(reader)?);
        }
        let count = reader.u8()? as usize;
        if count == 0 || count > MAX_BLOCKS {
            return Err(Error::Length);
        }
        let mut blocks = Vec::new();
        for _ in 0..count {
            blocks.push(reader.bytes(2_000_000)?.to_vec());
        }
        let witness = Self {
            before,
            ledger,
            spends,
            index,
            archive,
            blocks,
        };
        witness.validate()?;
        Ok(witness)
    }
}
