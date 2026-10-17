use crate::{Error, IndexProof, IndexSnapshot, Observation, Result};
use zutti_protocol::{Digest, policy::PaymentWindow};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateOutcome {
    Paid { height: u32 },
    ExpiredAbsent,
    Waiting,
}

pub fn classify_candidate(
    window: PaymentWindow,
    snapshot: IndexSnapshot,
    proof: &IndexProof,
    txid: Digest,
) -> Result<CandidateOutcome> {
    if window.index_start() != snapshot.context.start() || snapshot.height < window.anchor_height()
    {
        return Err(Error::Window);
    }
    match proof.verify(snapshot, txid)? {
        Observation::Present(height)
            if height > window.anchor_height() && height <= window.expiry() =>
        {
            Ok(CandidateOutcome::Paid { height })
        }
        Observation::Present(_) => Err(Error::Window),
        Observation::Absent if snapshot.height > window.expiry() => {
            Ok(CandidateOutcome::ExpiredAbsent)
        }
        Observation::Absent => Ok(CandidateOutcome::Waiting),
    }
}
