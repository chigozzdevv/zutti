use crate::{CandidateIndex, Error, IndexContext, IndexProof, IndexSnapshot, Result};
use std::sync::Arc;
use zutti_protocol::Digest;

#[derive(Debug, Clone)]
pub(crate) enum HistoryIndex {
    Full {
        index: Box<CandidateIndex>,
        recorded: Option<Vec<IndexProof>>,
    },
    Witness {
        snapshot: IndexSnapshot,
        proofs: Arc<[IndexProof]>,
        cursor: usize,
    },
}
impl HistoryIndex {
    pub(crate) fn new(context: IndexContext) -> Self {
        Self::Full {
            index: Box::new(CandidateIndex::new(context)),
            recorded: None,
        }
    }
    pub(crate) fn witness(snapshot: IndexSnapshot, proofs: Vec<IndexProof>) -> Self {
        Self::Witness {
            snapshot,
            proofs: proofs.into(),
            cursor: 0,
        }
    }
    pub(crate) fn snapshot(&self) -> IndexSnapshot {
        match self {
            Self::Full { index, .. } => index.snapshot(),
            Self::Witness { snapshot, .. } => *snapshot,
        }
    }
    pub(crate) fn append_block(&mut self, height: u32, txids: &[Digest]) -> Result<()> {
        match self {
            Self::Full { index, recorded } => {
                index.append_recorded(height, txids, recorded.as_mut())
            }
            Self::Witness {
                snapshot,
                proofs,
                cursor,
            } => {
                if snapshot.height.checked_add(1) != Some(height) {
                    return Err(Error::Height);
                }
                if txids.is_empty()
                    || txids.len() > 100_000
                    || txids.iter().any(|tx| tx.0 == [0; 32])
                {
                    return Err(Error::Transactions);
                }
                let mut next = *snapshot;
                next.height = height;
                let mut offset = *cursor;
                for txid in txids {
                    next.root = proofs
                        .get(offset)
                        .ok_or(Error::Path)?
                        .insert(next, *txid, height)?;
                    offset += 1;
                }
                *snapshot = next;
                *cursor = offset;
                Ok(())
            }
        }
    }
    pub(crate) fn prove(&self, txid: Digest) -> Result<IndexProof> {
        match self {
            Self::Full { index, .. } => Ok(index.prove(txid)),
            _ => Err(Error::Path),
        }
    }
    pub(crate) fn start_recording(&mut self) -> Result<()> {
        match self {
            Self::Full { recorded, .. } => {
                *recorded = Some(Vec::new());
                Ok(())
            }
            _ => Err(Error::Path),
        }
    }
    pub(crate) fn take_transcript(&mut self) -> Result<Vec<IndexProof>> {
        match self {
            Self::Full { recorded, .. } => recorded.take().ok_or(Error::Path),
            _ => Err(Error::Path),
        }
    }
    pub(crate) fn finish(&self) -> Result<()> {
        if let Self::Witness { proofs, cursor, .. } = self
            && *cursor != proofs.len()
        {
            return Err(Error::Path);
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn cloned_witnesses_validate_independent_index_transitions() {
        let context = IndexContext::new(Digest([1; 32]), Digest([2; 32]), 0).unwrap();
        let mut full = HistoryIndex::new(context);
        let before = full.snapshot();
        let first = [Digest([3; 32])];
        let second = [Digest([4; 32])];
        full.start_recording().unwrap();
        full.append_block(1, &first).unwrap();
        full.append_block(2, &second).unwrap();
        let mut left = HistoryIndex::witness(before, full.take_transcript().unwrap());
        let mut right = left.clone();
        assert!(left.append_block(1, &second).is_err());
        assert_eq!(left.snapshot(), before);
        left.append_block(1, &first).unwrap();
        left.append_block(2, &second).unwrap();
        left.finish().unwrap();
        assert_eq!(right.snapshot(), before);
        assert!(right.finish().is_err());
        right.append_block(1, &first).unwrap();
        right.append_block(2, &second).unwrap();
        right.finish().unwrap();
        assert_eq!(left.snapshot(), full.snapshot());
        assert_eq!(right.snapshot(), full.snapshot());
    }
}
