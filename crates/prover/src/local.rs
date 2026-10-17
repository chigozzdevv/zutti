use crate::{Error, ProofEnvelope, ProofVerifier, backend::receipt_codec};
use bincode::Options;
use risc0_zkvm::{ExecutorEnv, ExternalProver, Prover, ProverOpts, VerifierContext};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;
use zutti_protocol::statement::Statement;

#[derive(Debug)]
pub struct LocalProver {
    pub(crate) executable: PathBuf,
}
impl LocalProver {
    pub fn new(executable: impl AsRef<Path>) -> Self {
        Self {
            executable: executable.as_ref().to_path_buf(),
        }
    }
    pub(crate) fn prove(
        &self,
        bytes: Zeroizing<Vec<u8>>,
        statement: Statement,
        program: &[u8],
        session_cycles: u64,
        segment_po2: u32,
    ) -> Result<(Statement, ProofEnvelope), Error> {
        let length = u32::try_from(bytes.len())
            .map_err(|_| Error::Witness)?
            .to_le_bytes();
        let mut stdin = Zeroizing::new(Vec::with_capacity(bytes.len() + 4));
        stdin.extend_from_slice(&length);
        stdin.extend_from_slice(&bytes);
        self.prove_stdin(stdin, statement, program, session_cycles, segment_po2)
    }
    pub(crate) fn prove_stdin(
        &self,
        stdin: Zeroizing<Vec<u8>>,
        statement: Statement,
        program: &[u8],
        session_cycles: u64,
        segment_po2: u32,
    ) -> Result<(Statement, ProofEnvelope), Error> {
        let env = ExecutorEnv::builder()
            .write_slice(stdin.as_slice())
            .session_limit(Some(session_cycles))
            .segment_limit_po2(segment_po2)
            .build()
            .map_err(|_| Error::Proving)?;
        let info = ExternalProver::new("zutti-local", &self.executable)
            .prove_with_ctx(
                env,
                &VerifierContext::default().with_dev_mode(false),
                program,
                &ProverOpts::succinct().with_dev_mode(false),
            )
            .map_err(|_| Error::Proving)?;
        let receipt = receipt_codec()
            .serialize(&info.receipt)
            .map_err(|_| Error::Receipt)?;
        let envelope = ProofEnvelope::new(&statement, receipt)?;
        ProofVerifier.verify(&envelope, &statement)?;
        Ok((statement, envelope))
    }
}

#[derive(Debug, serde::Serialize)]
pub struct ExecutionMetrics {
    pub cycles: u64,
    pub segments: usize,
    pub seconds: f64,
    pub journal_matches: bool,
    pub proof_generated: bool,
}
