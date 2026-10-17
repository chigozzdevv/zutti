pub use crate::local::LocalProver;
use crate::program::{Program, ProgramKind};
use crate::{Error, ProofEnvelope};
use zeroize::Zeroizing;
use zutti_protocol::{
    Digest,
    encoding::Canonical,
    statement::{ProofKind, Statement},
};
use zutti_source::query::{MAX_QUERY_WITNESS, QueryClaim, QueryWitness};

pub fn query_image_id() -> Digest {
    let image = risc0_zkvm::sha::Digest::from(zutti_methods::HISTORY_QUERY_ID);
    Digest(image.into())
}

pub fn query_statement(claim: &QueryClaim) -> Result<Statement, Error> {
    query_statement_with(claim, &Program::current(ProgramKind::HistoryQuery))
}
pub fn query_statement_with(claim: &QueryClaim, program: &Program) -> Result<Statement, Error> {
    if program.kind() != ProgramKind::HistoryQuery {
        return Err(Error::Binding);
    }
    Ok(Statement::new(
        ProofKind::HistoryQuery,
        claim.domain(),
        program.image(),
        claim.to_bytes()?,
    )?)
}

impl LocalProver {
    pub fn prove_query(&self, witness: &QueryWitness) -> Result<(Statement, ProofEnvelope), Error> {
        self.prove_query_with(witness, &Program::current(ProgramKind::HistoryQuery))
    }
    pub fn prove_query_with(
        &self,
        witness: &QueryWitness,
        program: &Program,
    ) -> Result<(Statement, ProofEnvelope), Error> {
        let claim = witness.evaluate().map_err(|_| Error::Witness)?;
        let bytes = Zeroizing::new(witness.to_bytes()?);
        if bytes.is_empty() || bytes.len() > MAX_QUERY_WITNESS {
            return Err(Error::Witness);
        }
        self.prove(
            bytes,
            query_statement_with(&claim, program)?,
            program.bytes(),
            16_777_216,
            18,
        )
    }
}
