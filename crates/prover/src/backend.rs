use crate::preparation::preparation_image_id;
use crate::query::query_image_id;
use bincode::Options;
use risc0_zkvm::{InnerReceipt, Receipt, VerifierContext};
use zutti_protocol::{
    Digest,
    encoding::{Canonical, Reader, Writer},
    statement::{ProofKind, Statement},
};
use zutti_source::query::QueryClaim;
use zutti_wallet::preparation::PreparationClaim;

pub const MAX_PROOF: usize = 16 * 1024 * 1024;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error(transparent)]
    Protocol(#[from] zutti_protocol::Error),
    #[error("proof identity, domain, kind or statement mismatch")]
    Binding,
    #[error("no cryptographic verifier admitted for this proof kind and program")]
    BackendUnavailable,
    #[error("invalid cryptographic receipt")]
    Receipt,
    #[error("development receipts are not accepted")]
    DevelopmentReceipt,
    #[error("receipt journal does not match the expected claim")]
    Journal,
    #[error("local proof generation failed")]
    Proving,
    #[error("private proof witness is invalid")]
    Witness,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ProofEnvelope {
    kind: ProofKind,
    domain: Digest,
    program: Digest,
    statement: Digest,
    proof: Vec<u8>,
}
impl core::fmt::Debug for ProofEnvelope {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ProofEnvelope")
            .field("kind", &self.kind)
            .field("bytes", &self.proof.len())
            .finish_non_exhaustive()
    }
}
impl ProofEnvelope {
    pub fn proof(&self) -> &[u8] {
        &self.proof
    }
    pub fn new(statement: &Statement, proof: Vec<u8>) -> Result<Self, Error> {
        if proof.is_empty() || proof.len() > MAX_PROOF {
            return Err(zutti_protocol::Error::Length.into());
        }
        Ok(Self {
            kind: statement.kind(),
            domain: statement.domain(),
            program: statement.program(),
            statement: statement.digest()?,
            proof,
        })
    }
    pub fn check_binding(&self, expected: &Statement) -> Result<(), Error> {
        if self.kind != expected.kind()
            || self.domain != expected.domain()
            || self.program != expected.program()
            || self.statement != expected.digest()?
        {
            return Err(Error::Binding);
        }
        Ok(())
    }
}
impl Canonical for ProofEnvelope {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        writer.fixed(b"ZUTTIPRF");
        writer.u16(1);
        writer.u8(self.kind as u8);
        writer.fixed(&self.domain.0);
        writer.fixed(&self.program.0);
        writer.fixed(&self.statement.0);
        writer.bytes(&self.proof)
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTIPRF" || reader.u16()? != 1 {
            return Err(zutti_protocol::Error::Version);
        }
        let kind = ProofKind::from_id(reader.u8()?)?;
        let domain = Digest::nonzero(reader.fixed()?)?;
        let program = Digest::nonzero(reader.fixed()?)?;
        let statement = Digest::nonzero(reader.fixed()?)?;
        let proof = reader.bytes(MAX_PROOF)?.to_vec();
        if proof.is_empty() {
            return Err(zutti_protocol::Error::Length);
        }
        Ok(Self {
            kind,
            domain,
            program,
            statement,
            proof,
        })
    }
}

#[derive(Debug)]
pub struct VerifiedProof {
    statement: Digest,
    kind: ProofKind,
}
impl VerifiedProof {
    pub const fn statement(&self) -> Digest {
        self.statement
    }
    pub const fn kind(&self) -> ProofKind {
        self.kind
    }
}

#[derive(Debug, Default)]
pub struct ProofVerifier;
impl ProofVerifier {
    pub fn verify(
        &self,
        envelope: &ProofEnvelope,
        expected: &Statement,
    ) -> Result<VerifiedProof, Error> {
        envelope.check_binding(expected)?;
        let (domain, image) = match expected.kind() {
            ProofKind::HistoryQuery if expected.program() == query_image_id() => (
                QueryClaim::from_bytes(expected.payload())?.domain(),
                zutti_methods::HISTORY_QUERY_ID,
            ),
            ProofKind::Preparation if expected.program() == preparation_image_id() => (
                PreparationClaim::from_bytes(expected.payload())?.domain(),
                zutti_methods::PREPARATION_ID,
            ),
            _ => return Err(Error::BackendUnavailable),
        };
        if domain != expected.domain() {
            return Err(Error::Binding);
        }
        let receipt: Receipt = receipt_codec()
            .deserialize(&envelope.proof)
            .map_err(|_| Error::Receipt)?;
        match receipt.inner {
            InnerReceipt::Fake(_) => return Err(Error::DevelopmentReceipt),
            InnerReceipt::Succinct(_) | InnerReceipt::Groth16(_) => {}
            _ => return Err(Error::Receipt),
        }
        receipt
            .verify_with_context(&VerifierContext::default().with_dev_mode(false), image)
            .map_err(|_| Error::Receipt)?;
        if receipt.journal.bytes != expected.payload() {
            return Err(Error::Journal);
        }
        Ok(VerifiedProof {
            statement: expected.digest()?,
            kind: expected.kind(),
        })
    }
}

pub(crate) fn receipt_codec() -> impl Options {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(MAX_PROOF as u64)
        .reject_trailing_bytes()
}
