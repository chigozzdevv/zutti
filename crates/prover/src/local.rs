use crate::{Error, ProofEnvelope, ProofVerifier, backend::receipt_codec};
use bincode::Options;
use risc0_zkvm::{ExecutorEnv, ExternalProver, Prover, ProverOpts, VerifierContext};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;
use zutti_protocol::statement::Statement;

#[derive(Debug)]
pub struct LocalProver {
    executable: PathBuf,
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
        let env = ExecutorEnv::builder()
            .write_slice(&length)
            .write_slice(bytes.as_slice())
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
