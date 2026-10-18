use crate::{
    artifact::ProofArtifact,
    source::{SourceJob, SourcePrograms, groth16_seal},
};
use std::path::Path;
use zutti_protocol::{
    encoding::Canonical,
    statement::{ProofKind, Statement},
};
use zutti_source::{
    proof::{SourceInput, SourcePolicy, SourceReceiptClaim, SourceWitness},
    range::{SourceRangeClaim, SourceRangeWitness},
    storage::{RangeLimits, RangeStore},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
#[derive(Debug)]
pub struct SourceBootstrap {
    store: RangeStore,
    programs: SourcePrograms,
}
impl SourceBootstrap {
    pub fn create(
        path: &Path,
        genesis: Vec<u8>,
        policy: SourcePolicy,
        programs: SourcePrograms,
        limits: RangeLimits,
    ) -> Result<Self> {
        check_policy(policy, &programs)?;
        Ok(Self {
            store: RangeStore::create(path, genesis, policy, limits)?,
            programs,
        })
    }
    pub fn open(
        path: &Path,
        policy: SourcePolicy,
        programs: SourcePrograms,
        limits: RangeLimits,
    ) -> Result<Self> {
        check_policy(policy, &programs)?;
        let bootstrap = Self {
            store: RangeStore::open(path, policy, limits)?,
            programs,
        };
        bootstrap.verify_retained()?;
        Ok(bootstrap)
    }
    pub fn from_store(store: RangeStore, programs: SourcePrograms) -> Result<Self> {
        check_policy(store.policy(), &programs)?;
        let bootstrap = Self { store, programs };
        bootstrap.verify_retained()?;
        Ok(bootstrap)
    }
    pub const fn store(&self) -> &RangeStore {
        &self.store
    }
    pub fn genesis_job(&self) -> Result<SourceJob> {
        Ok(SourceJob::transition_with(
            &SourceWitness {
                policy: self.store.policy(),
                input: SourceInput::Genesis(self.store.genesis().to_vec()),
            },
            None,
            &self.programs,
        )?)
    }
    pub fn preview(&self, blocks: Vec<Vec<u8>>) -> Result<SourceJob> {
        Ok(SourceJob::range(
            &self.store.preview(blocks)?,
            &[],
            &self.programs,
        )?)
    }
    pub fn advance(&mut self, blocks: Vec<Vec<u8>>) -> Result<u64> {
        self.preview(blocks.clone())?;
        Ok(self.store.advance(blocks)?)
    }
    pub fn range_job(&self, id: u64) -> Result<SourceJob> {
        let witness = self
            .store
            .batch(id)?
            .witness
            .as_ref()
            .ok_or("source range already has retained evidence")?;
        Ok(SourceJob::range(witness, &[], &self.programs)?)
    }
    pub fn accept_range(&mut self, id: u64, artifact: &ProofArtifact) -> Result<()> {
        let batch = self.store.batch(id)?;
        if let Some(bytes) = &batch.evidence {
            self.verify_artifact(artifact, ProofKind::SourceRange, &batch.claim.to_bytes()?)?;
            if *bytes == artifact.to_bytes()? {
                return Ok(());
            }
            return Err("source range already has different retained evidence".into());
        }
        let job = self.range_job(id)?;
        artifact.verify(job.statement())?;
        groth16_seal(artifact, job.statement())?;
        let claim = SourceRangeClaim::from_bytes(job.statement().payload())?;
        Ok(self.store.retain_range(id, &claim, artifact.to_bytes()?)?)
    }
    pub fn aggregate_job(&self, ids: &[u64]) -> Result<SourceJob> {
        self.store.aggregation(ids)?;
        let artifacts = ids
            .iter()
            .map(|id| self.range_artifact(*id))
            .collect::<Result<Vec<_>>>()?;
        let witness = SourceRangeWitness::Aggregate {
            policy: self.store.policy(),
            ranges: ids
                .iter()
                .map(|id| Ok(self.store.batch(*id)?.claim.clone()))
                .collect::<Result<Vec<_>>>()?,
        };
        Ok(SourceJob::range(
            &witness,
            &artifacts.iter().collect::<Vec<_>>(),
            &self.programs,
        )?)
    }
    pub fn accept_aggregate(&mut self, ids: &[u64], artifact: &ProofArtifact) -> Result<()> {
        if let Some(first) = ids.first() {
            let batch = self.store.batch(*first)?;
            if batch.evidence.as_deref() == Some(artifact.to_bytes()?.as_slice()) {
                self.verify_artifact(artifact, ProofKind::SourceRange, &batch.claim.to_bytes()?)?;
                return Ok(());
            }
        }
        let job = self.aggregate_job(ids)?;
        artifact.verify(job.statement())?;
        groth16_seal(artifact, job.statement())?;
        let claim = SourceRangeClaim::from_bytes(job.statement().payload())?;
        Ok(self
            .store
            .retain_aggregate(ids, &claim, artifact.to_bytes()?)?)
    }
    pub fn fold_job(&self, id: u64) -> Result<SourceJob> {
        let prefix = ProofArtifact::from_bytes(
            self.store
                .evidence()
                .ok_or("source bootstrap has no genesis-anchored prefix proof")?,
        )?;
        let range = self.range_artifact(id)?;
        let witness = SourceWitness {
            policy: self.store.policy(),
            input: SourceInput::Fold {
                previous: Box::new(self.store.base().clone()),
                range: Box::new(self.store.batch(id)?.claim.clone()),
            },
        };
        Ok(SourceJob::fold(&witness, &prefix, &range, &self.programs)?)
    }
    pub fn accept_prefix(&mut self, artifact: &ProofArtifact) -> Result<()> {
        let claim = SourceReceiptClaim::from_bytes(artifact.statement().payload())?;
        self.verify_artifact(artifact, ProofKind::SourceTransition, &claim.to_bytes()?)?;
        Ok(self.store.retain_prefix(&claim, artifact.to_bytes()?)?)
    }
    fn range_artifact(&self, id: u64) -> Result<ProofArtifact> {
        let batch = self.store.batch(id)?;
        let artifact = ProofArtifact::from_bytes(
            batch
                .evidence
                .as_deref()
                .ok_or("source range proof is unavailable")?,
        )?;
        self.verify_artifact(&artifact, ProofKind::SourceRange, &batch.claim.to_bytes()?)?;
        Ok(artifact)
    }
    fn verify_artifact(
        &self,
        artifact: &ProofArtifact,
        kind: ProofKind,
        payload: &[u8],
    ) -> Result<()> {
        let policy = self.store.policy();
        let expected = Statement::new(kind, policy.id()?, policy.program, payload.to_vec())?;
        artifact.verify(&expected)?;
        groth16_seal(artifact, &expected)?;
        Ok(())
    }
    fn verify_retained(&self) -> Result<()> {
        if let Some(bytes) = self.store.evidence() {
            let artifact = ProofArtifact::from_bytes(bytes)?;
            self.verify_artifact(
                &artifact,
                ProofKind::SourceTransition,
                &self.store.base().to_bytes()?,
            )?;
        }
        for batch in self.store.batches() {
            if batch.evidence.is_some() {
                self.range_artifact(batch.id)?;
            }
        }
        Ok(())
    }
}
fn check_policy(policy: SourcePolicy, programs: &SourcePrograms) -> Result<()> {
    if policy != programs.policy(policy.network, policy.index_start, policy.confirmations) {
        return Err("source program pair differs from its bootstrap policy".into());
    }
    policy.to_bytes()?;
    Ok(())
}
