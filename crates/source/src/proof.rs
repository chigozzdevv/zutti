use crate::{
    IndexSnapshot,
    bootstrap::{GenesisBlock, genesis_hash},
    state::{SourceCandidate, TransitionError},
    transition::{StateSnapshot, TransitionWitness},
};
use zcash_protocol::consensus::NetworkType;
use zutti_protocol::{
    Digest, Error,
    commitment::hash,
    encoding::{Canonical, Reader, Writer},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourcePolicy {
    pub program: Digest,
    pub registry_program: Digest,
    pub network: NetworkType,
    pub index_start: u32,
    pub confirmations: u32,
}
impl SourcePolicy {
    pub fn id(&self) -> zutti_protocol::Result<Digest> {
        Ok(hash(b"source-policy", &[&self.to_bytes()?]))
    }
}
impl Canonical for SourcePolicy {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        Digest::nonzero(self.program.0)?;
        Digest::nonzero(self.registry_program.0)?;
        if self.confirmations == 0 || self.confirmations > 10_000 {
            return Err(Error::Invalid("source confirmations"));
        }
        writer.fixed(b"ZUTTISPL");
        writer.u16(1);
        writer.fixed(&self.program.0);
        writer.fixed(&self.registry_program.0);
        writer.u8(match self.network {
            NetworkType::Main => 1,
            NetworkType::Test => 2,
            _ => return Err(Error::Invalid("source network")),
        });
        writer.u32(self.index_start);
        writer.u32(self.confirmations);
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTISPL" || reader.u16()? != 1 {
            return Err(Error::Version);
        }
        let program = Digest::nonzero(reader.fixed()?)?;
        let registry_program = Digest::nonzero(reader.fixed()?)?;
        let network = match reader.u8()? {
            1 => NetworkType::Main,
            2 => NetworkType::Test,
            _ => return Err(Error::Invalid("source network")),
        };
        let policy = Self {
            program,
            registry_program,
            network,
            index_start: reader.u32()?,
            confirmations: reader.u32()?,
        };
        policy.to_bytes()?;
        Ok(policy)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceReceiptClaim {
    pub program: Digest,
    pub genesis: Digest,
    pub policy: Digest,
    pub state: Digest,
    pub height: u32,
    pub tip: Digest,
    pub index: IndexSnapshot,
    pub work: [u8; 32],
    pub archive: Digest,
}
impl SourceReceiptClaim {
    pub(crate) fn from_state(
        policy: SourcePolicy,
        state: &StateSnapshot,
    ) -> Result<Self, TransitionError> {
        if state.headers.network() != policy.network
            || state.index.context.rules() != policy.id()?
            || state.index.context.start() != policy.index_start
        {
            return Err(Error::Invalid("source policy context").into());
        }
        Ok(Self {
            program: policy.program,
            genesis: genesis_hash(policy.network)?,
            policy: policy.id()?,
            state: state.commitment()?,
            height: state.height()?,
            tip: state.tip()?,
            index: state.index(),
            work: state.work()?,
            archive: state.archive(),
        })
    }
}
impl Canonical for SourceReceiptClaim {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        for digest in [
            self.program,
            self.genesis,
            self.policy,
            self.state,
            self.tip,
        ] {
            Digest::nonzero(digest.0)?;
        }
        if self.index.context.genesis() != self.genesis
            || self.index.context.rules() != self.policy
            || self.index.height != self.height.max(self.index.context.start())
            || self.work == [0; 32]
        {
            return Err(Error::Invalid("source receipt context"));
        }
        writer.fixed(b"ZUTTISRC");
        writer.u16(2);
        for digest in [self.program, self.genesis, self.policy, self.state] {
            writer.fixed(&digest.0);
        }
        writer.u32(self.height);
        writer.fixed(&self.tip.0);
        self.index.encode(writer)?;
        writer.fixed(&self.work);
        writer.fixed(&Digest::nonzero(self.archive.0)?.0);
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTISRC" || reader.u16()? != 2 {
            return Err(Error::Version);
        }
        let claim = Self {
            program: Digest(reader.fixed()?),
            genesis: Digest(reader.fixed()?),
            policy: Digest(reader.fixed()?),
            state: Digest(reader.fixed()?),
            height: reader.u32()?,
            tip: Digest(reader.fixed()?),
            index: IndexSnapshot::decode(reader)?,
            work: reader.fixed()?,
            archive: Digest::nonzero(reader.fixed()?)?,
        };
        claim.to_bytes()?;
        Ok(claim)
    }
}

#[derive(Debug, Clone)]
pub enum SourceInput {
    Genesis(Vec<u8>),
    Advance {
        previous: Box<SourceReceiptClaim>,
        transition: Box<TransitionWitness>,
    },
}
#[derive(Debug, Clone)]
pub struct SourceWitness {
    pub policy: SourcePolicy,
    pub input: SourceInput,
}
impl SourceWitness {
    pub fn evaluate(&self) -> Result<SourceReceiptClaim, TransitionError> {
        let state = match &self.input {
            SourceInput::Genesis(bytes) => {
                let genesis = GenesisBlock::parse(bytes, self.policy.network)?;
                SourceCandidate::from_genesis(&genesis, self.policy.id()?, self.policy.index_start)?
                    .state_snapshot()?
            }
            SourceInput::Advance {
                previous,
                transition,
            } => {
                let expected = SourceReceiptClaim::from_state(self.policy, transition.before())?;
                if **previous != expected {
                    return Err(Error::Invalid("recursive source predecessor").into());
                }
                transition.evaluate()?
            }
        };
        SourceReceiptClaim::from_state(self.policy, &state)
    }
    pub fn assumption(&self) -> Option<&SourceReceiptClaim> {
        match &self.input {
            SourceInput::Advance { previous, .. } => Some(previous.as_ref()),
            _ => None,
        }
    }
}
impl Canonical for SourceWitness {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        writer.fixed(b"ZUTTISWT");
        writer.u16(1);
        self.policy.encode(writer)?;
        match &self.input {
            SourceInput::Genesis(bytes) if !bytes.is_empty() && bytes.len() <= 2_000_000 => {
                writer.u8(0);
                writer.bytes(bytes)?;
            }
            SourceInput::Genesis(_) => return Err(Error::Length),
            SourceInput::Advance {
                previous,
                transition,
            } => {
                writer.u8(1);
                previous.encode(writer)?;
                transition.encode(writer)?;
            }
        }
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTISWT" || reader.u16()? != 1 {
            return Err(Error::Version);
        }
        let policy = SourcePolicy::decode(reader)?;
        let input = match reader.u8()? {
            0 => SourceInput::Genesis(reader.bytes(2_000_000)?.to_vec()),
            1 => SourceInput::Advance {
                previous: Box::new(SourceReceiptClaim::decode(reader)?),
                transition: Box::new(TransitionWitness::decode(reader)?),
            },
            _ => return Err(Error::Invalid("source witness kind")),
        };
        Ok(Self { policy, input })
    }
}
