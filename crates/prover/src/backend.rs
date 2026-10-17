use crate::preparation::preparation_image_id;
use crate::query::query_image_id;
use bincode::Options;
use risc0_zkvm::{InnerReceipt, Receipt, VerifierContext};
use zutti_protocol::pool::{PoolClaim, SourceUpdateClaim};
use zutti_protocol::{
    Digest,
    encoding::{Canonical, Reader, Writer},
    statement::{ProofKind, Statement},
};
use zutti_source::{proof::SourceReceiptClaim, query::QueryClaim};
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
            ProofKind::HistoryQuery if admitted_program(expected.kind(), expected.program()) => (
                QueryClaim::from_bytes(expected.payload())?.domain(),
                risc0_zkvm::sha::Digest::from(expected.program().0),
            ),
            ProofKind::Preparation if admitted_program(expected.kind(), expected.program()) => (
                PreparationClaim::from_bytes(expected.payload())?.domain(),
                risc0_zkvm::sha::Digest::from(expected.program().0),
            ),
            ProofKind::SourceTransition
                if admitted_program(expected.kind(), expected.program()) =>
            {
                let claim = SourceReceiptClaim::from_bytes(expected.payload())?;
                if claim.program != expected.program() {
                    return Err(Error::Binding);
                }
                (
                    claim.policy,
                    risc0_zkvm::sha::Digest::from(expected.program().0),
                )
            }
            ProofKind::Source if admitted_program(expected.kind(), expected.program()) => (
                SourceUpdateClaim::from_bytes(expected.payload())?.policy,
                risc0_zkvm::sha::Digest::from(expected.program().0),
            ),
            ProofKind::Note | ProofKind::Reservation | ProofKind::Settlement
                if admitted_program(expected.kind(), expected.program()) =>
            {
                let claim = PoolClaim::from_bytes(expected.payload())?;
                if crate::pool::proof_kind(claim.kind) != expected.kind() {
                    return Err(Error::Binding);
                }
                (
                    claim.domain,
                    risc0_zkvm::sha::Digest::from(expected.program().0),
                )
            }
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

const PREPARATION_V1: Digest = Digest([
    0xf9, 0x7a, 0xeb, 0x93, 0xb4, 0x06, 0x64, 0x60, 0xd3, 0x72, 0xb4, 0x33, 0xa8, 0x9b, 0xd0, 0x7d,
    0x6b, 0xe3, 0x09, 0x34, 0xa5, 0x5f, 0xa5, 0xde, 0x6a, 0x5e, 0x4c, 0x42, 0x1d, 0x9b, 0x39, 0x16,
]);
const HISTORY_QUERY_V1: Digest = Digest([
    0x81, 0xc2, 0x5a, 0x3f, 0x06, 0x8f, 0x64, 0xe4, 0x36, 0x64, 0x44, 0x56, 0xf6, 0xf4, 0xd3, 0x05,
    0xb8, 0x09, 0x8b, 0xc4, 0x10, 0xd6, 0xa2, 0xed, 0x4a, 0xd5, 0x2a, 0xab, 0xb8, 0x4e, 0xa9, 0x0b,
]);

fn admitted_program(kind: ProofKind, image: Digest) -> bool {
    match kind {
        ProofKind::Preparation => image == preparation_image_id() || image == PREPARATION_V1,
        ProofKind::HistoryQuery => image == query_image_id() || image == HISTORY_QUERY_V1,
        ProofKind::SourceTransition => {
            image == crate::source::source_image_id() || image == SOURCE_V1
        }
        ProofKind::Source => image == crate::source::update_image_id(),
        ProofKind::Note | ProofKind::Reservation | ProofKind::Settlement => {
            image == crate::pool::pool_image_id() || image == POOL_V1
        }
    }
}

pub(crate) fn receipt_codec() -> impl Options {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(MAX_PROOF as u64)
        .reject_trailing_bytes()
}

const SOURCE_V1: Digest = Digest([
    0xc5, 0xa1, 0xd8, 0x68, 0xe1, 0x76, 0x64, 0x79, 0x66, 0xce, 0x6e, 0x72, 0x88, 0x60, 0xc9, 0x22,
    0x5c, 0x99, 0xe0, 0x5a, 0x0d, 0x88, 0x80, 0xa4, 0x74, 0xe1, 0xae, 0xeb, 0x65, 0x2e, 0xc6, 0x69,
]);

const POOL_V1: Digest = Digest([
    0x33, 0xe5, 0xd5, 0x57, 0xb9, 0xab, 0x66, 0x53, 0x70, 0x85, 0x93, 0x81, 0xd0, 0x80, 0x85, 0xe5,
    0xdb, 0xb7, 0x90, 0xef, 0xd8, 0x1b, 0x22, 0x04, 0x9e, 0x57, 0x38, 0xe0, 0x36, 0xc4, 0xb6, 0x02,
]);
