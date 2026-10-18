use crate::{Error, artifact::ProofArtifact};
use zutti_protocol::{
    Digest,
    encoding::{Canonical, Reader, Writer},
    pool::SourceUpdateClaim,
    statement::ProofKind,
};
use zutti_source::{
    bootstrap::genesis_hash, checkpoint::SourceCheckpoint, proof::SourcePolicy,
    transition::StateSnapshot,
};

#[derive(Debug, Clone)]
pub struct SourceAdmission {
    pub policy: SourcePolicy,
    pub minimum_work: [u8; 32],
    pub minimum_height: u32,
    pub maximum_age: u64,
    pub maximum_future: u64,
}
impl SourceAdmission {
    fn validate(&self) -> Result<(), Error> {
        self.policy.to_bytes()?;
        if !crate::backend::admitted_source_pair(self.policy.program, self.policy.registry_program)
            || self.minimum_work == [0; 32]
            || self.minimum_height == 0
            || self.maximum_age == 0
            || self.maximum_age > 86_400
            || self.maximum_future > 7_200
        {
            return Err(Error::Binding);
        }
        Ok(())
    }
    pub fn verify(
        &self,
        artifact: &ProofArtifact,
        state: &StateSnapshot,
        checkpoint: SourceCheckpoint,
        now: u64,
    ) -> Result<VerifiedSource, Error> {
        self.validate()?;
        zutti_source::upgrade::ensure_supported(
            self.policy.network,
            state.height().map_err(|_| Error::Binding)?,
        )
        .map_err(|_| Error::Binding)?;
        let statement = artifact.statement();
        let claim = SourceUpdateClaim::from_bytes(statement.payload())?;
        if statement.kind() != ProofKind::Source
            || statement.program() != self.policy.registry_program
            || statement.domain() != self.policy.id()?
            || claim.genesis != genesis_hash(self.policy.network).map_err(|_| Error::Binding)?
            || claim.policy != self.policy.id()?
            || claim.state != state.commitment()?
            || claim.checkpoint != checkpoint.id()?
            || claim.work_be != state.work().map_err(|_| Error::Binding)?
            || claim.work_be < self.minimum_work
            || state.height().map_err(|_| Error::Binding)? < self.minimum_height
            || checkpoint.archive != state.archive()
            || checkpoint.stable.index.context != state.index().context
            || checkpoint.stable.index.context.start() != self.policy.index_start
            || checkpoint.stable.index.context.rules() != self.policy.id()?
            || checkpoint.stable.index.height != checkpoint.stable.height
            || checkpoint
                .stable
                .height
                .checked_add(self.policy.confirmations)
                != Some(state.height().map_err(|_| Error::Binding)?)
            || checkpoint.stable.work > claim.work_be
        {
            return Err(Error::Binding);
        }
        artifact.verify(statement)?;
        let verified = VerifiedSource {
            checkpoint,
            state: claim.state,
            height: state.height().map_err(|_| Error::Binding)?,
            timestamp: u64::from(state.timestamp().map_err(|_| Error::Binding)?),
            maximum_age: self.maximum_age,
            maximum_future: self.maximum_future,
        };
        verified.checkpoint(now)?;
        Ok(verified)
    }
}
impl Canonical for SourceAdmission {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        self.validate()
            .map_err(|_| zutti_protocol::Error::Invalid("source admission"))?;
        writer.fixed(b"ZUTTIADM");
        writer.u16(1);
        self.policy.encode(writer)?;
        writer.fixed(&self.minimum_work);
        writer.u32(self.minimum_height);
        writer.u64(self.maximum_age);
        writer.u64(self.maximum_future);
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTIADM" || reader.u16()? != 1 {
            return Err(zutti_protocol::Error::Version);
        }
        let profile = Self {
            policy: SourcePolicy::decode(reader)?,
            minimum_work: reader.fixed()?,
            minimum_height: reader.u32()?,
            maximum_age: reader.u64()?,
            maximum_future: reader.u64()?,
        };
        profile
            .validate()
            .map_err(|_| zutti_protocol::Error::Invalid("source admission"))?;
        Ok(profile)
    }
}

#[derive(Debug)]
pub struct VerifiedSource {
    checkpoint: SourceCheckpoint,
    state: Digest,
    height: u32,
    timestamp: u64,
    maximum_age: u64,
    maximum_future: u64,
}
impl VerifiedSource {
    pub fn checkpoint(&self, now: u64) -> Result<&SourceCheckpoint, Error> {
        if self.timestamp > now.saturating_add(self.maximum_future)
            || now.saturating_sub(self.timestamp) > self.maximum_age
        {
            return Err(Error::StaleSource);
        }
        Ok(&self.checkpoint)
    }
    pub const fn state(&self) -> Digest {
        self.state
    }
    pub const fn height(&self) -> u32 {
        self.height
    }
    pub const fn timestamp(&self) -> u64 {
        self.timestamp
    }
}
