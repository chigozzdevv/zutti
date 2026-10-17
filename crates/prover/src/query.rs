pub use crate::local::LocalProver;
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
    Ok(Statement::new(
        ProofKind::HistoryQuery,
        claim.domain(),
        query_image_id(),
        claim.to_bytes()?,
    )?)
}

impl LocalProver {
    pub fn prove_query(&self, witness: &QueryWitness) -> Result<(Statement, ProofEnvelope), Error> {
        let claim = witness.evaluate().map_err(|_| Error::Witness)?;
        let bytes = Zeroizing::new(witness.to_bytes()?);
        if bytes.is_empty() || bytes.len() > MAX_QUERY_WITNESS {
            return Err(Error::Witness);
        }
        self.prove(
            bytes,
            query_statement(&claim)?,
            zutti_methods::HISTORY_QUERY_ELF,
            16_777_216,
            18,
        )
    }
}
