use crate::{Error, ProofEnvelope, local::LocalProver};
use zeroize::Zeroizing;
use zutti_protocol::{
    Digest,
    encoding::Canonical,
    pool::{OperationKind, PoolClaim},
    statement::{ProofKind, Statement},
};
use zutti_wallet::pool::{MAX_POOL_WITNESS, PoolWitness};

pub fn pool_image_id() -> Digest {
    Digest(risc0_zkvm::sha::Digest::from(zutti_methods::POOL_ID).into())
}
pub fn proof_kind(kind: OperationKind) -> ProofKind {
    match kind {
        OperationKind::Deposit | OperationKind::Transfer | OperationKind::Withdraw => {
            ProofKind::Note
        }
        OperationKind::Reserve => ProofKind::Reservation,
        OperationKind::Settle => ProofKind::Settlement,
    }
}
pub fn pool_statement(claim: &PoolClaim) -> Result<Statement, Error> {
    Ok(Statement::new(
        proof_kind(claim.kind),
        claim.domain,
        pool_image_id(),
        claim.to_bytes()?,
    )?)
}
impl LocalProver {
    pub fn prove_pool(&self, witness: &PoolWitness) -> Result<(Statement, ProofEnvelope), Error> {
        let claim = witness.evaluate()?;
        let bytes = Zeroizing::new(witness.to_bytes()?);
        if bytes.len() > MAX_POOL_WITNESS {
            return Err(Error::Witness);
        }
        self.prove(
            bytes,
            pool_statement(&claim)?,
            zutti_methods::POOL_ELF,
            268_435_456,
            20,
        )
    }
    pub fn execute_pool(
        &self,
        witness: &PoolWitness,
    ) -> Result<crate::local::ExecutionMetrics, Box<dyn std::error::Error>> {
        use risc0_zkvm::{Executor, ExecutorEnv, ExternalProver};
        let claim = witness.evaluate()?;
        let bytes = Zeroizing::new(witness.to_bytes()?);
        if bytes.len() > MAX_POOL_WITNESS {
            return Err(Error::Witness.into());
        }
        let env = ExecutorEnv::builder()
            .write_slice(&(bytes.len() as u32).to_le_bytes())
            .write_slice(bytes.as_slice())
            .session_limit(Some(268_435_456))
            .segment_limit_po2(20)
            .build()?;
        let started = std::time::Instant::now();
        let session = ExternalProver::new("zutti-pool-executor", &self.executable)
            .execute(env, zutti_methods::POOL_ELF)?;
        if session.journal.bytes != claim.to_bytes()? {
            return Err(Error::Journal.into());
        }
        Ok(crate::local::ExecutionMetrics {
            cycles: session.cycles(),
            segments: session.segments.len(),
            seconds: started.elapsed().as_secs_f64(),
            journal_matches: true,
            proof_generated: false,
        })
    }
}
