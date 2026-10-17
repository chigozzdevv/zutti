use crate::{Error, IndexSnapshot, Result};

#[derive(Debug)]
pub struct VerifiedHistory {
    snapshot: IndexSnapshot,
}
impl VerifiedHistory {
    pub const fn snapshot(&self) -> IndexSnapshot {
        self.snapshot
    }
}

pub fn verify_history(_candidate: IndexSnapshot, _proof: &[u8]) -> Result<VerifiedHistory> {
    Err(Error::ConsensusUnavailable)
}
