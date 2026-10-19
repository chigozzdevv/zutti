use crate::{
    Error, ProofEnvelope, ProofVerifier,
    artifact::ProofArtifact,
    backend::receipt_codec,
    local::LocalProver,
    program::{Program, ProgramKind},
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
    proof::{SourceInput, SourcePolicy, SourceReceiptClaim, SourceWitness},
    range::{MAX_RANGE_INPUTS, SourceRangeClaim, SourceRangeWitness},
    transition::MAX_WITNESS,
    update::SourceUpdateWitness,
};

pub const MAX_SOURCE_CYCLES: u64 = 2_147_483_648;
pub const MAX_DIAGNOSTIC_CYCLES: u64 = 1 << 38;
const MAX_SOURCE_STDIN: usize = MAX_WITNESS + 4 + MAX_RANGE_INPUTS * 260;

#[derive(Debug, serde::Serialize)]
pub struct SourceExecutionComparison {
    #[serde(flatten)]
    pub execution: crate::local::ExecutionMetrics,
    pub expected_program: String,
    pub executed_program: String,
    pub program_matches_statement: bool,
    pub cycle_limit: u64,
}

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
    source_statement_with(claim, &Program::current(ProgramKind::Source))
}
fn source_statement_with(
    claim: &SourceReceiptClaim,
    program: &Program,
) -> Result<Statement, Error> {
    if program.kind() != ProgramKind::Source || claim.program != program.image() {
        return Err(Error::Binding);
    }
    Ok(Statement::new(
        ProofKind::SourceTransition,
        claim.policy,
        program.image(),
        claim.to_bytes()?,
    )?)
}
pub fn update_statement(claim: &SourceUpdateClaim) -> Result<Statement, Error> {
    update_statement_with(claim, &Program::current(ProgramKind::SourceUpdate))
}

pub fn range_statement(claim: &SourceRangeClaim) -> Result<Statement, Error> {
    range_statement_with(claim, &Program::current(ProgramKind::Source))
}
fn range_statement_with(claim: &SourceRangeClaim, program: &Program) -> Result<Statement, Error> {
    if program.kind() != ProgramKind::Source || claim.end.program != program.image() {
        return Err(Error::Binding);
    }
    Ok(Statement::new(
        ProofKind::SourceRange,
        claim.end.policy,
        program.image(),
        claim.to_bytes()?,
    )?)
}
fn update_statement_with(claim: &SourceUpdateClaim, program: &Program) -> Result<Statement, Error> {
    if program.kind() != ProgramKind::SourceUpdate {
        return Err(Error::Binding);
    }
    Ok(Statement::new(
        ProofKind::Source,
        claim.policy,
        program.image(),
        claim.to_bytes()?,
    )?)
}
#[derive(Debug, Clone)]
pub struct SourcePrograms {
    source: Program,
    update: Program,
}
impl SourcePrograms {
    pub fn current() -> Self {
        Self {
            source: Program::current(ProgramKind::Source),
            update: Program::current(ProgramKind::SourceUpdate),
        }
    }
    pub fn new(source: Program, update: Program) -> Result<Self, Error> {
        if source.kind() != ProgramKind::Source
            || update.kind() != ProgramKind::SourceUpdate
            || !crate::backend::admitted_source_pair(source.image(), update.image())
        {
            return Err(Error::Binding);
        }
        Ok(Self { source, update })
    }
    pub fn read(directory: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self::new(
            Program::read(ProgramKind::Source, &directory.join("source/program.elf"))?,
            Program::read(
                ProgramKind::SourceUpdate,
                &directory.join("source-update/program.elf"),
            )?,
        )?)
    }
    pub fn policy(
        &self,
        network: zcash_protocol::consensus::NetworkType,
        index_start: u32,
        confirmations: u32,
    ) -> SourcePolicy {
        SourcePolicy {
            program: self.source.image(),
            registry_program: self.update.image(),
            network,
            index_start,
            confirmations,
        }
    }
    fn check(&self, policy: SourcePolicy) -> Result<(), Error> {
        if policy.program != self.source.image() || policy.registry_program != self.update.image() {
            return Err(Error::Binding);
        }
        policy.to_bytes()?;
        Ok(())
    }
    fn check_range(&self, policy: SourcePolicy) -> Result<(), Error> {
        self.check(policy)?;
        if !crate::backend::admitted_range_program(self.source.image()) {
            return Err(Error::BackendUnavailable);
        }
        Ok(())
    }
    fn check_batch(&self, witness: &SourceWitness) -> Result<(), Error> {
        if let SourceInput::Advance { transition, .. } = &witness.input
            && transition.block_count() > crate::backend::source_block_limit(self.source.image())
        {
            return Err(Error::Witness);
        }
        Ok(())
    }
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
    code: Program,
}
impl SourceJob {
    pub fn read_public(directory: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let statement = Statement::from_bytes(&zutti_wallet::backup::read_private(
            &directory.join("statement.bin"),
            66_560,
        )?)?;
        let kind = match statement.kind() {
            ProofKind::Source => ProgramKind::SourceUpdate,
            ProofKind::SourceTransition | ProofKind::SourceRange => ProgramKind::Source,
            _ => return Err(Error::Binding.into()),
        };
        let code = Program::read(kind, &directory.join("program.elf"))?;
        let update = match statement.kind() {
            ProofKind::Source => {
                if update_statement_with(
                    &SourceUpdateClaim::from_bytes(statement.payload())?,
                    &code,
                )? != statement
                {
                    return Err(Error::Binding.into());
                }
                true
            }
            ProofKind::SourceTransition => {
                if source_statement_with(
                    &SourceReceiptClaim::from_bytes(statement.payload())?,
                    &code,
                )? != statement
                {
                    return Err(Error::Binding.into());
                }
                false
            }
            ProofKind::SourceRange => {
                if range_statement_with(&SourceRangeClaim::from_bytes(statement.payload())?, &code)?
                    != statement
                {
                    return Err(Error::Binding.into());
                }
                false
            }
            _ => return Err(Error::Binding.into()),
        };
        let stdin =
            zutti_wallet::backup::read_private(&directory.join("stdin.bin"), MAX_SOURCE_STDIN)?;
        if stdin.len() < 4 {
            return Err(Error::Witness.into());
        }
        Ok(Self {
            statement,
            stdin,
            update,
            code,
        })
    }
    pub fn transition(
        witness: &SourceWitness,
        previous: Option<&ProofArtifact>,
    ) -> Result<Self, Error> {
        Self::transition_with(witness, previous, &SourcePrograms::current())
    }
    pub fn transition_with(
        witness: &SourceWitness,
        previous: Option<&ProofArtifact>,
        programs: &SourcePrograms,
    ) -> Result<Self, Error> {
        programs.check(witness.policy)?;
        programs.check_batch(witness)?;
        if matches!(witness.input, SourceInput::Fold { .. }) {
            return Err(Error::Binding);
        }
        let claim = witness.evaluate().map_err(|_| Error::Witness)?;
        let mut stdin = framed(witness)?;
        match (witness.assumption(), previous) {
            (Some(claim), Some(artifact)) => stdin.extend_from_slice(&groth16_seal(
                artifact,
                &source_statement_with(claim, &programs.source)?,
            )?),
            (None, None) => {}
            _ => return Err(Error::Binding),
        }
        Ok(Self {
            statement: source_statement_with(&claim, &programs.source)?,
            stdin,
            update: false,
            code: programs.source.clone(),
        })
    }
    pub fn range(
        witness: &SourceRangeWitness,
        children: &[&ProofArtifact],
        programs: &SourcePrograms,
    ) -> Result<Self, Error> {
        programs.check_range(witness.policy())?;
        if let SourceRangeWitness::Transition(witness) = witness {
            programs.check_batch(witness)?;
        }
        let claim = witness.evaluate().map_err(|_| Error::Witness)?;
        let mut stdin = framed(witness)?;
        let assumptions = witness.assumptions();
        if assumptions.len() != children.len() {
            return Err(Error::Binding);
        }
        for (claim, proof) in assumptions.iter().zip(children) {
            stdin.extend_from_slice(&groth16_seal(
                proof,
                &range_statement_with(claim, &programs.source)?,
            )?);
        }
        Ok(Self {
            statement: range_statement_with(&claim, &programs.source)?,
            stdin,
            update: false,
            code: programs.source.clone(),
        })
    }
    pub fn fold(
        witness: &SourceWitness,
        prefix: &ProofArtifact,
        range_proof: &ProofArtifact,
        programs: &SourcePrograms,
    ) -> Result<Self, Error> {
        programs.check_range(witness.policy)?;
        let SourceInput::Fold { previous, range } = &witness.input else {
            return Err(Error::Binding);
        };
        let claim = witness.evaluate().map_err(|_| Error::Witness)?;
        let mut stdin = framed(witness)?;
        stdin.extend_from_slice(&groth16_seal(
            prefix,
            &source_statement_with(previous, &programs.source)?,
        )?);
        stdin.extend_from_slice(&groth16_seal(
            range_proof,
            &range_statement_with(range, &programs.source)?,
        )?);
        Ok(Self {
            statement: source_statement_with(&claim, &programs.source)?,
            stdin,
            update: false,
            code: programs.source.clone(),
        })
    }
    pub fn update(
        witness: &SourceUpdateWitness,
        chain: &ProofArtifact,
        previous: Option<&ProofArtifact>,
    ) -> Result<Self, Error> {
        Self::update_with(witness, chain, previous, &SourcePrograms::current())
    }
    pub fn update_with(
        witness: &SourceUpdateWitness,
        chain: &ProofArtifact,
        previous: Option<&ProofArtifact>,
        programs: &SourcePrograms,
    ) -> Result<Self, Error> {
        programs.check(witness.policy)?;
        let claim = witness.evaluate().map_err(|_| Error::Witness)?;
        let mut stdin = framed(witness)?;
        stdin.extend_from_slice(&groth16_seal(
            chain,
            &source_statement_with(&witness.chain, &programs.source)?,
        )?);
        match (&witness.previous, previous) {
            (Some(update), Some(artifact)) => stdin.extend_from_slice(&groth16_seal(
                artifact,
                &update_statement_with(&update.claim, &programs.update)?,
            )?),
            (None, None) => {}
            _ => return Err(Error::Binding),
        }
        Ok(Self {
            statement: update_statement_with(&claim, &programs.update)?,
            stdin,
            update: true,
            code: programs.update.clone(),
        })
    }
    pub fn id(&self) -> Result<Digest, Error> {
        Ok(zutti_protocol::commitment::hash(
            b"source-job",
            &[
                &self.statement.to_bytes()?,
                self.stdin.as_slice(),
                &[u8::from(self.update)],
                self.program(),
            ],
        ))
    }
    pub fn matches(&self, other: &Self) -> bool {
        self.statement == other.statement
            && self.stdin == other.stdin
            && self.update == other.update
            && self.program() == other.program()
    }
    pub const fn statement(&self) -> &Statement {
        &self.statement
    }
    fn program(&self) -> &[u8] {
        self.code.bytes()
    }
    pub fn export_public(
        &self,
        directory: &Path,
        cycles: u64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if cycles == 0 || cycles > MAX_SOURCE_CYCLES {
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
    let mut framed = Zeroizing::new(Vec::with_capacity(bytes.len() + 4 + MAX_RANGE_INPUTS * 260));
    framed.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    framed.extend_from_slice(&bytes);
    Ok(framed)
}

impl LocalProver {
    pub fn execute_source(
        &self,
        job: &SourceJob,
    ) -> Result<crate::local::ExecutionMetrics, Box<dyn std::error::Error>> {
        self.execute_source_profiled(job, None)
    }
    pub fn execute_source_profiled(
        &self,
        job: &SourceJob,
        profile: Option<&Path>,
    ) -> Result<crate::local::ExecutionMetrics, Box<dyn std::error::Error>> {
        self.execute_source_program(job, job.program(), profile, MAX_SOURCE_CYCLES)
    }
    pub fn compare_source_execution(
        &self,
        job: &SourceJob,
        candidate: &Path,
        profile: &Path,
    ) -> Result<SourceExecutionComparison, Box<dyn std::error::Error>> {
        self.compare_source_execution_with_limit(job, candidate, Some(profile), MAX_SOURCE_CYCLES)
    }
    pub fn compare_source_execution_with_limit(
        &self,
        job: &SourceJob,
        candidate: &Path,
        profile: Option<&Path>,
        cycle_limit: u64,
    ) -> Result<SourceExecutionComparison, Box<dyn std::error::Error>> {
        use std::io::Read;
        if !(1..=MAX_DIAGNOSTIC_CYCLES).contains(&cycle_limit) {
            return Err(Error::Binding.into());
        }
        let file = std::fs::File::open(candidate)?;
        if !file.metadata()?.is_file() {
            return Err("candidate program must be a regular file".into());
        }
        let mut bytes = Vec::new();
        file.take(crate::program::MAX_PROGRAM as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.is_empty() || bytes.len() > crate::program::MAX_PROGRAM {
            return Err(Error::Binding.into());
        }
        let executed = Digest(risc0_zkvm::compute_image_id(&bytes)?.into());
        Ok(SourceExecutionComparison {
            execution: self.execute_source_program(job, &bytes, profile, cycle_limit)?,
            expected_program: job.statement.program().to_string(),
            executed_program: executed.to_string(),
            program_matches_statement: executed == job.statement.program(),
            cycle_limit,
        })
    }
    fn execute_source_program(
        &self,
        job: &SourceJob,
        program: &[u8],
        profile: Option<&Path>,
        cycle_limit: u64,
    ) -> Result<crate::local::ExecutionMetrics, Box<dyn std::error::Error>> {
        let mut builder = ExecutorEnv::builder();
        builder
            .write_slice(job.stdin.as_slice())
            .session_limit(Some(cycle_limit))
            .segment_limit_po2(20);
        if let Some(path) = profile {
            builder.enable_profiler(path);
        }
        let env = builder.build()?;
        let started = std::time::Instant::now();
        let session = ExternalProver::new("zutti-source-executor", &self.executable)
            .execute(env, program)
            .map_err(|error| -> Box<dyn std::error::Error> {
                if error
                    .chain()
                    .any(|cause| cause.to_string().starts_with("Session limit exceeded: "))
                {
                    Error::ExecutionLimit.into()
                } else {
                    error.into()
                }
            })?;
        match session.exit_code {
            risc0_zkvm::ExitCode::Halted(0) => {}
            risc0_zkvm::ExitCode::SessionLimit => return Err(Error::ExecutionLimit.into()),
            _ => return Err(Error::Proving.into()),
        }
        if session.journal.bytes != job.statement.payload() {
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
    pub fn prove_source(&self, job: &SourceJob) -> Result<(Statement, ProofEnvelope), Error> {
        self.prove_stdin(
            Zeroizing::new(job.stdin.to_vec()),
            job.statement.clone(),
            job.program(),
            MAX_SOURCE_CYCLES,
            20,
        )
    }
}

fn persist_public(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    crate::artifact::persist_bounded(path, bytes, MAX_WITNESS + 524)
}
