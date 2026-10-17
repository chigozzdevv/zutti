use crate::{
    Error, ProofEnvelope, ProofVerifier, artifact::ProofArtifact, backend::receipt_codec,
    local::LocalProver,
};
use bincode::Options;
use risc0_zkvm::{Executor, ExecutorEnv, ExternalProver, InnerReceipt, Receipt, sha::Digestible};
use std::path::Path;
use zeroize::Zeroizing;
use zutti_protocol::{
    Digest,
    encoding::Canonical,
    pool::SourceUpdateClaim,
    statement::{ProofKind, Statement},
};
use zutti_source::{
    proof::{SourcePolicy, SourceReceiptClaim, SourceWitness},
    transition::MAX_WITNESS,
    update::SourceUpdateWitness,
};

pub fn source_image_id() -> Digest {
    Digest(risc0_zkvm::sha::Digest::from(zutti_methods::SOURCE_ID).into())
}
pub fn update_image_id() -> Digest {
    Digest(risc0_zkvm::sha::Digest::from(zutti_methods::SOURCE_UPDATE_ID).into())
}
pub fn policy(
    network: zcash_protocol::consensus::NetworkType,
    index_start: u32,
    confirmations: u32,
) -> SourcePolicy {
    SourcePolicy {
        program: source_image_id(),
        registry_program: update_image_id(),
        network,
        index_start,
        confirmations,
    }
}
pub fn source_statement(claim: &SourceReceiptClaim) -> Result<Statement, Error> {
    if claim.program != source_image_id() {
        return Err(Error::Binding);
    }
    Ok(Statement::new(
        ProofKind::SourceTransition,
        claim.policy,
        source_image_id(),
        claim.to_bytes()?,
    )?)
}
pub fn update_statement(claim: &SourceUpdateClaim) -> Result<Statement, Error> {
    Ok(Statement::new(
        ProofKind::Source,
        claim.policy,
        update_image_id(),
        claim.to_bytes()?,
    )?)
}
fn check_policy(policy: SourcePolicy) -> Result<(), Error> {
    if policy.program != source_image_id() || policy.registry_program != update_image_id() {
        return Err(Error::Binding);
    }
    policy.to_bytes()?;
    Ok(())
}
pub fn groth16_seal(artifact: &ProofArtifact, expected: &Statement) -> Result<Vec<u8>, Error> {
    ProofVerifier.verify(artifact.proof(), expected)?;
    let receipt: Receipt = receipt_codec()
        .deserialize(artifact.proof().proof())
        .map_err(|_| Error::Receipt)?;
    let InnerReceipt::Groth16(receipt) = receipt.inner else {
        return Err(Error::Receipt);
    };
    if receipt.seal.len() != 256 {
        return Err(Error::Receipt);
    }
    let parameters = risc0_zkvm::Groth16ReceiptVerifierParameters::default().digest();
    if receipt.verifier_parameters != parameters {
        return Err(Error::Receipt);
    }
    let mut seal = parameters.as_bytes()[..4].to_vec();
    seal.extend_from_slice(&receipt.seal);
    Ok(seal)
}

#[derive(Debug)]
pub struct SourceJob {
    statement: Statement,
    stdin: Zeroizing<Vec<u8>>,
    update: bool,
}
impl SourceJob {
    pub fn read_public(directory: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let statement = Statement::from_bytes(&zutti_wallet::backup::read_private(
            &directory.join("statement.bin"),
            66_560,
        )?)?;
        let update = match statement.kind() {
            ProofKind::Source => {
                if update_statement(&SourceUpdateClaim::from_bytes(statement.payload())?)?
                    != statement
                {
                    return Err(Error::Binding.into());
                }
                true
            }
            ProofKind::SourceTransition => {
                if source_statement(&SourceReceiptClaim::from_bytes(statement.payload())?)?
                    != statement
                {
                    return Err(Error::Binding.into());
                }
                false
            }
            _ => return Err(Error::Binding.into()),
        };
        let stdin =
            zutti_wallet::backup::read_private(&directory.join("stdin.bin"), MAX_WITNESS + 524)?;
        if stdin.len() < 4 {
            return Err(Error::Witness.into());
        }
        Ok(Self {
            statement,
            stdin,
            update,
        })
    }
    pub fn transition(
        witness: &SourceWitness,
        previous: Option<&ProofArtifact>,
    ) -> Result<Self, Error> {
        check_policy(witness.policy)?;
        let claim = witness.evaluate().map_err(|_| Error::Witness)?;
        let mut stdin = framed(witness)?;
        match (witness.assumption(), previous) {
            (Some(claim), Some(artifact)) => {
                stdin.extend_from_slice(&groth16_seal(artifact, &source_statement(claim)?)?)
            }
            (None, None) => {}
            _ => return Err(Error::Binding),
        }
        Ok(Self {
            statement: source_statement(&claim)?,
            stdin,
            update: false,
        })
    }
    pub fn update(
        witness: &SourceUpdateWitness,
        chain: &ProofArtifact,
        previous: Option<&ProofArtifact>,
    ) -> Result<Self, Error> {
        check_policy(witness.policy)?;
        let claim = witness.evaluate().map_err(|_| Error::Witness)?;
        let mut stdin = framed(witness)?;
        stdin.extend_from_slice(&groth16_seal(chain, &source_statement(&witness.chain)?)?);
        match (&witness.previous, previous) {
            (Some(update), Some(artifact)) => {
                stdin.extend_from_slice(&groth16_seal(artifact, &update_statement(&update.claim)?)?)
            }
            (None, None) => {}
            _ => return Err(Error::Binding),
        }
        Ok(Self {
            statement: update_statement(&claim)?,
            stdin,
            update: true,
        })
    }
    pub const fn statement(&self) -> &Statement {
        &self.statement
    }
    fn program(&self) -> &'static [u8] {
        if self.update {
            zutti_methods::SOURCE_UPDATE_ELF
        } else {
            zutti_methods::SOURCE_ELF
        }
    }
    pub fn export_public(
        &self,
        directory: &Path,
        cycles: u64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if cycles == 0 || cycles > 268_435_456 {
            return Err("public source job exceeds the configured cycle bound".into());
        }
        std::fs::create_dir(directory)?;
        let directory = directory.canonicalize()?;
        for (name, bytes) in [
            ("program.elf", self.program()),
            ("stdin.bin", self.stdin.as_slice()),
            ("journal.bin", self.statement.payload()),
            ("statement.bin", &self.statement.to_bytes()?),
        ] {
            persist_public(&directory.join(name), bytes)?;
        }
        let job = serde_json::json!({"version":1,"visibility":"public","image_id":self.statement.program().to_string(),"program":directory.join("program.elf"),"stdin":directory.join("stdin.bin"),"journal":directory.join("journal.bin"),"cycles":cycles});
        persist_public(
            &directory.join("job.json"),
            &serde_json::to_vec_pretty(&job)?,
        )?;
        if self.update {
            let claim = SourceUpdateClaim::from_bytes(self.statement.payload())?;
            let value = serde_json::json!({"genesis":claim.genesis.to_string(),"policy":claim.policy.to_string(),"previous":claim.previous.to_string(),"previous_stable":claim.previous_stable.to_string(),"state":claim.state.to_string(),"checkpoint":claim.checkpoint.to_string(),"work_be":hex::encode(claim.work_be)});
            persist_public(
                &directory.join("claim.json"),
                &serde_json::to_vec_pretty(&value)?,
            )?;
        }
        Ok(())
    }
}
fn framed(value: &impl Canonical) -> Result<Zeroizing<Vec<u8>>, Error> {
    let bytes = value.to_bytes()?;
    if bytes.is_empty() || bytes.len() > MAX_WITNESS {
        return Err(Error::Witness);
    }
    let mut framed = Zeroizing::new(Vec::with_capacity(bytes.len() + 524));
    framed.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    framed.extend_from_slice(&bytes);
    Ok(framed)
}

#[derive(Debug, serde::Serialize)]
pub struct SourceExecution {
    pub cycles: u64,
    pub segments: usize,
    pub seconds: f64,
    pub journal_matches: bool,
    pub proof_generated: bool,
}
impl LocalProver {
    pub fn execute_source(
        &self,
        job: &SourceJob,
    ) -> Result<SourceExecution, Box<dyn std::error::Error>> {
        let env = ExecutorEnv::builder()
            .write_slice(job.stdin.as_slice())
            .session_limit(Some(2_147_483_648))
            .segment_limit_po2(20)
            .build()?;
        let started = std::time::Instant::now();
        let session = ExternalProver::new("zutti-source-executor", &self.executable)
            .execute(env, job.program())?;
        if session.journal.bytes != job.statement.payload() {
            return Err(Error::Journal.into());
        }
        Ok(SourceExecution {
            cycles: session.cycles(),
            segments: session.segments.len(),
            seconds: started.elapsed().as_secs_f64(),
            journal_matches: true,
            proof_generated: false,
        })
    }
    pub fn prove_source(&self, job: &SourceJob) -> Result<(Statement, ProofEnvelope), Error> {
        self.prove_stdin(
            Zeroizing::new(job.stdin.to_vec()),
            job.statement.clone(),
            job.program(),
            2_147_483_648,
            20,
        )
    }
}

fn persist_public(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    use std::{fs::File, io::Write};
    if bytes.is_empty() || bytes.len() > MAX_WITNESS + 524 {
        return Err("source artifact exceeds its size bound".into());
    }
    let parent = path.parent().ok_or("missing source artifact directory")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist_noclobber(path)
        .map_err(|error| error.error)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}
