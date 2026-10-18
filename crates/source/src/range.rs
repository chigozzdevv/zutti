use crate::{
    proof::{SourceInput, SourcePolicy, SourceReceiptClaim, SourceWitness},
    state::TransitionError,
    transition::TransitionWitness,
};
use zutti_protocol::{
    Error,
    encoding::{Canonical, Reader, Writer},
};

pub const MAX_RANGE_INPUTS: usize = 4;
pub const RANGE_WITNESS_MAGIC: &[u8; 8] = b"ZUTTIRGW";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRangeClaim {
    pub start: SourceReceiptClaim,
    pub end: SourceReceiptClaim,
}
impl SourceRangeClaim {
    pub fn new(start: SourceReceiptClaim, end: SourceReceiptClaim) -> zutti_protocol::Result<Self> {
        let claim = Self { start, end };
        claim.validate()?;
        Ok(claim)
    }
    fn validate(&self) -> zutti_protocol::Result<()> {
        self.start.to_bytes()?;
        self.end.to_bytes()?;
        if self.start.program != self.end.program
            || self.start.genesis != self.end.genesis
            || self.start.policy != self.end.policy
            || self.start.index.context != self.end.index.context
            || self.start.height >= self.end.height
            || self.start.work >= self.end.work
            || self.start.state == self.end.state
        {
            return Err(Error::Invalid("source range endpoints"));
        }
        Ok(())
    }
    pub fn require_policy(&self, policy: SourcePolicy) -> Result<(), TransitionError> {
        self.validate()?;
        if self.start.program != policy.program
            || self.start.policy != policy.id()?
            || self.start.genesis != crate::bootstrap::genesis_hash(policy.network)?
            || self.start.index.context.start() != policy.index_start
        {
            return Err(Error::Invalid("source range policy").into());
        }
        Ok(())
    }
    pub fn aggregate(policy: SourcePolicy, claims: &[Self]) -> Result<Self, TransitionError> {
        if !(2..=MAX_RANGE_INPUTS).contains(&claims.len()) {
            return Err(Error::Length.into());
        }
        for claim in claims {
            claim.require_policy(policy)?;
        }
        if claims.windows(2).any(|pair| pair[0].end != pair[1].start) {
            return Err(Error::Invalid("nonconsecutive source ranges").into());
        }
        Ok(Self::new(
            claims[0].start.clone(),
            claims[claims.len() - 1].end.clone(),
        )?)
    }
}
impl Canonical for SourceRangeClaim {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        self.validate()?;
        writer.fixed(b"ZUTTIRNG");
        writer.u16(1);
        self.start.encode(writer)?;
        self.end.encode(writer)
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTIRNG" || reader.u16()? != 1 {
            return Err(Error::Version);
        }
        Self::new(
            SourceReceiptClaim::decode(reader)?,
            SourceReceiptClaim::decode(reader)?,
        )
    }
}

#[derive(Debug, Clone)]
pub enum SourceRangeWitness {
    Transition(Box<SourceWitness>),
    Aggregate {
        policy: SourcePolicy,
        ranges: Vec<SourceRangeClaim>,
    },
}
impl SourceRangeWitness {
    pub fn transition(
        policy: SourcePolicy,
        transition: TransitionWitness,
    ) -> Result<Self, TransitionError> {
        let previous = SourceReceiptClaim::from_state(policy, transition.before())?;
        Ok(Self::Transition(Box::new(SourceWitness {
            policy,
            input: SourceInput::Advance {
                previous: Box::new(previous),
                transition: Box::new(transition),
            },
        })))
    }
    pub const fn policy(&self) -> SourcePolicy {
        match self {
            Self::Transition(witness) => witness.policy,
            Self::Aggregate { policy, .. } => *policy,
        }
    }
    pub fn evaluate(&self) -> Result<SourceRangeClaim, TransitionError> {
        match self {
            Self::Transition(witness) => {
                let SourceInput::Advance { previous, .. } = &witness.input else {
                    return Err(Error::Invalid("source range transition").into());
                };
                let claim = SourceRangeClaim::new((**previous).clone(), witness.evaluate()?)?;
                claim.require_policy(witness.policy)?;
                Ok(claim)
            }
            Self::Aggregate { policy, ranges } => SourceRangeClaim::aggregate(*policy, ranges),
        }
    }
    pub fn assumptions(&self) -> &[SourceRangeClaim] {
        match self {
            Self::Transition(_) => &[],
            Self::Aggregate { ranges, .. } => ranges,
        }
    }
}
impl Canonical for SourceRangeWitness {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        writer.fixed(RANGE_WITNESS_MAGIC);
        writer.u16(1);
        match self {
            Self::Transition(witness) => {
                if !matches!(witness.input, SourceInput::Advance { .. }) {
                    return Err(Error::Invalid("source range transition"));
                }
                writer.u8(0);
                witness.encode(writer)?;
            }
            Self::Aggregate { policy, ranges } => {
                if !(2..=MAX_RANGE_INPUTS).contains(&ranges.len()) {
                    return Err(Error::Length);
                }
                writer.u8(1);
                policy.encode(writer)?;
                writer.u8(ranges.len() as u8);
                for range in ranges {
                    range.encode(writer)?;
                }
            }
        }
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if &reader.fixed::<8>()? != RANGE_WITNESS_MAGIC || reader.u16()? != 1 {
            return Err(Error::Version);
        }
        match reader.u8()? {
            0 => {
                let witness = SourceWitness::decode(reader)?;
                if !matches!(witness.input, SourceInput::Advance { .. }) {
                    return Err(Error::Invalid("source range transition"));
                }
                Ok(Self::Transition(Box::new(witness)))
            }
            1 => {
                let policy = SourcePolicy::decode(reader)?;
                let count = reader.u8()? as usize;
                if !(2..=MAX_RANGE_INPUTS).contains(&count) {
                    return Err(Error::Length);
                }
                let mut ranges = Vec::with_capacity(count);
                for _ in 0..count {
                    ranges.push(SourceRangeClaim::decode(reader)?);
                }
                Ok(Self::Aggregate { policy, ranges })
            }
            _ => Err(Error::Invalid("source range witness kind")),
        }
    }
}
