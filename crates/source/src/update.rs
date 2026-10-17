use crate::{
    checkpoint::{HeaderPath, SourceCheckpoint},
    proof::{SourcePolicy, SourceReceiptClaim},
    state::TransitionError,
};
use zutti_protocol::{
    Digest, Error,
    encoding::{Canonical, Reader, Writer},
    pool::SourceUpdateClaim,
};

#[derive(Debug, Clone)]
pub struct PreviousUpdate {
    pub claim: SourceUpdateClaim,
    pub checkpoint: SourceCheckpoint,
    pub ancestry: HeaderPath,
}
#[derive(Debug, Clone)]
pub struct SourceUpdateWitness {
    pub policy: SourcePolicy,
    pub chain: SourceReceiptClaim,
    pub stable: HeaderPath,
    pub previous: Option<Box<PreviousUpdate>>,
}
impl SourceUpdateWitness {
    pub fn evaluate(&self) -> Result<SourceUpdateClaim, TransitionError> {
        let genesis = crate::bootstrap::genesis_hash(self.policy.network)?;
        let policy = self.policy.id()?;
        if self.chain.program != self.policy.program
            || self.chain.genesis != genesis
            || self.chain.policy != policy
            || self.chain.index.context.start() != self.policy.index_start
        {
            return Err(Error::Invalid("source update profile").into());
        }
        self.chain.to_bytes()?;
        let height = self
            .chain
            .height
            .checked_sub(self.policy.confirmations)
            .filter(|h| *h >= self.policy.index_start)
            .ok_or(Error::Invalid("stable source height"))?;
        let stable = self
            .stable
            .verify(genesis, policy, self.chain.archive, height)?;
        let checkpoint = SourceCheckpoint {
            stable,
            archive: self.chain.archive,
        };
        let (previous, previous_stable) = match &self.previous {
            None => (Digest::default(), Digest::default()),
            Some(previous) => {
                previous.claim.to_bytes()?;
                let old = &previous.checkpoint;
                if previous.claim.genesis != genesis
                    || previous.claim.policy != policy
                    || previous.claim.checkpoint != old.id()?
                    || old.stable.index.context.genesis() != genesis
                    || old.stable.index.context.rules() != policy
                    || old.stable.height > checkpoint.stable.height
                    || previous.claim.work_be >= self.chain.work
                    || previous.claim.state == self.chain.state
                {
                    return Err(Error::Invalid("source fork progression").into());
                }
                let ancestor = previous.ancestry.verify(
                    genesis,
                    policy,
                    self.chain.archive,
                    old.stable.height,
                )?;
                if ancestor != old.stable {
                    return Err(Error::Invalid("accepted stable source ancestry").into());
                }
                (previous.claim.state, previous.claim.checkpoint)
            }
        };
        Ok(SourceUpdateClaim {
            genesis,
            policy,
            previous,
            previous_stable,
            state: self.chain.state,
            checkpoint: checkpoint.id()?,
            work_be: self.chain.work,
        })
    }
}
impl Canonical for SourceUpdateWitness {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        writer.fixed(b"ZUTTISUW");
        writer.u16(1);
        self.policy.encode(writer)?;
        self.chain.encode(writer)?;
        self.stable.encode(writer)?;
        match &self.previous {
            None => writer.u8(0),
            Some(previous) => {
                writer.u8(1);
                previous.claim.encode(writer)?;
                previous.checkpoint.encode(writer)?;
                previous.ancestry.encode(writer)?;
            }
        }
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTISUW" || reader.u16()? != 1 {
            return Err(Error::Version);
        }
        let policy = SourcePolicy::decode(reader)?;
        let chain = SourceReceiptClaim::decode(reader)?;
        let stable = HeaderPath::decode(reader)?;
        let previous = match reader.u8()? {
            0 => None,
            1 => Some(Box::new(PreviousUpdate {
                claim: SourceUpdateClaim::decode(reader)?,
                checkpoint: SourceCheckpoint::decode(reader)?,
                ancestry: HeaderPath::decode(reader)?,
            })),
            _ => return Err(Error::Invalid("previous source update flag")),
        };
        Ok(Self {
            policy,
            chain,
            stable,
            previous,
        })
    }
}
