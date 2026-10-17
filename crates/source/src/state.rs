use crate::{
    CandidateIndex, Error, IndexContext, IndexProof, IndexSnapshot, Result, block::ParsedBlock,
};
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
