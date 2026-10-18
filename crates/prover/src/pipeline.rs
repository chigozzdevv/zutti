use crate::{
    artifact::ProofArtifact,
    source::{SourceJob, SourcePrograms},
};
use std::path::Path;
use zutti_protocol::{encoding::Canonical, statement::ProofKind};
use zutti_source::{
    proof::{SourceInput, SourcePolicy, SourceReceiptClaim},
    storage::SourceStore,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Debug)]
pub struct SourcePipeline {
    store: SourceStore,
    programs: SourcePrograms,
}
impl SourcePipeline {
    pub fn create(
        path: &Path,
        genesis: Vec<u8>,
        policy: SourcePolicy,
        programs: SourcePrograms,
    ) -> Result<Self> {
        check_policy(policy, &programs)?;
        Ok(Self {
            store: SourceStore::create(path, genesis, policy)?,
            programs,
        })
    }
    pub fn open(path: &Path, policy: SourcePolicy, programs: SourcePrograms) -> Result<Self> {
        check_policy(policy, &programs)?;
        let pipeline = Self {
            store: SourceStore::open(path, policy)?,
            programs,
        };
        pipeline.verify_retained()?;
        Ok(pipeline)
    }
    pub const fn store(&self) -> &SourceStore {
        &self.store
    }
    pub fn preview(&self, blocks: Vec<Vec<u8>>) -> Result<SourceJob> {
        self.verify_retained()?;
        if self.store.pending().is_some() {
            return Err("a source batch is already pending".into());
        }
        let predecessor = ProofArtifact::from_bytes(
            self.store
                .evidence()
                .ok_or("source predecessor proof is unavailable")?,
        )?;
        let previous = SourceReceiptClaim::from_bytes(predecessor.statement().payload())?;
        let mut candidate = self.store.candidate().clone();
        let transition = candidate.record_transition(blocks)?;
        let witness = zutti_source::proof::SourceWitness {
            policy: self.store.policy(),
            input: SourceInput::Advance {
                previous: Box::new(previous),
                transition: Box::new(transition),
            },
        };
        Ok(SourceJob::transition_with(
            &witness,
            Some(&predecessor),
            &self.programs,
        )?)
    }
    pub fn advance(&mut self, blocks: Vec<Vec<u8>>) -> Result<()> {
        self.verify_retained()?;
        if self.store.evidence().is_none() {
            return Err("source predecessor proof is unavailable".into());
        }
        Ok(self.store.advance(blocks)?)
    }
    pub fn job(&self) -> Result<SourceJob> {
        self.verify_retained()?;
        let pending = self.store.pending().ok_or("no source batch is pending")?;
        let predecessor = self
            .store
            .evidence()
            .map(ProofArtifact::from_bytes)
            .transpose()?;
        Ok(SourceJob::transition_with(
            pending,
            predecessor.as_ref(),
            &self.programs,
        )?)
    }
    pub fn accept(&mut self, artifact: &ProofArtifact) -> Result<()> {
        let expected = self.job()?;
        artifact.verify(expected.statement())?;
        crate::source::groth16_seal(artifact, expected.statement())?;
        let claim = SourceReceiptClaim::from_bytes(expected.statement().payload())?;
        Ok(self
            .store
            .retain_evidence(claim.state, artifact.to_bytes()?)?)
    }
    fn verify_retained(&self) -> Result<()> {
        let Some(bytes) = self.store.evidence() else {
            if matches!(
                self.store.pending().map(|w| &w.input),
                Some(SourceInput::Genesis(_))
            ) {
                return Ok(());
            }
            return Err("source predecessor proof is unavailable".into());
        };
        let artifact = ProofArtifact::from_bytes(bytes)?;
        let statement = artifact.statement();
        let policy = self.store.policy();
        if statement.kind() != ProofKind::SourceTransition
            || statement.program() != policy.program
            || statement.domain() != policy.id()?
        {
            return Err("stored proof has a different source program".into());
        }
        let claim = SourceReceiptClaim::from_bytes(statement.payload())?;
        if claim.policy != policy.id()?
            || claim.program != policy.program
            || claim.genesis != zutti_source::bootstrap::genesis_hash(policy.network)?
        {
            return Err("stored proof has a different source policy".into());
        }
        match self.store.pending().map(|w| &w.input) {
            Some(SourceInput::Advance { previous, .. }) if **previous == claim => {}
            None if claim.state == self.store.candidate().state_snapshot()?.commitment()? => {}
            _ => return Err("stored proof does not authenticate the source state".into()),
        }
        artifact.verify(statement)?;
        crate::source::groth16_seal(&artifact, statement)?;
        Ok(())
    }
}
fn check_policy(policy: SourcePolicy, programs: &SourcePrograms) -> Result<()> {
    if policy != programs.policy(policy.network, policy.index_start, policy.confirmations) {
        return Err("source program pair differs from its policy".into());
    }
    policy.to_bytes()?;
    Ok(())
}
