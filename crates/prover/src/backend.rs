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
use zutti_source::{proof::SourceReceiptClaim, query::QueryClaim, range::SourceRangeClaim};
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
    #[error("source evidence is stale or too far in the future")]
    StaleSource,
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
                if claim.program != expected.program()
                    || !source_height_supported(claim.program, claim.genesis, claim.height)
                {
                    return Err(Error::Binding);
                }
                (
                    claim.policy,
                    risc0_zkvm::sha::Digest::from(expected.program().0),
                )
            }
            ProofKind::SourceRange if admitted_program(expected.kind(), expected.program()) => {
                let claim = SourceRangeClaim::from_bytes(expected.payload())?;
                if claim.end.program != expected.program()
                    || !source_height_supported(
                        claim.end.program,
                        claim.end.genesis,
                        claim.end.height,
                    )
                {
                    return Err(Error::Binding);
                }
                (
                    claim.end.policy,
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

pub(crate) fn admitted_program(kind: ProofKind, image: Digest) -> bool {
    match kind {
        ProofKind::Preparation => {
            image == preparation_image_id() || image == PREPARATION_V1 || image == PREPARATION_V2
        }
        ProofKind::HistoryQuery => {
            image == query_image_id() || image == HISTORY_QUERY_V1 || image == HISTORY_QUERY_V2
        }
        ProofKind::SourceRange => admitted_range_program(image),
        ProofKind::SourceTransition => {
            image == crate::source::source_image_id()
                || image == SOURCE_V1
                || image == SOURCE_V2
                || image == SOURCE_V3
                || image == SOURCE_V4
                || image == SOURCE_V5
        }
        ProofKind::Source => {
            image == crate::source::update_image_id()
                || image == SOURCE_UPDATE_V1
                || image == SOURCE_UPDATE_V2
                || image == SOURCE_UPDATE_V3
                || image == SOURCE_UPDATE_V4
        }
        ProofKind::Note | ProofKind::Reservation | ProofKind::Settlement => {
            image == crate::pool::pool_image_id() || image == POOL_V1 || image == POOL_V2
        }
    }
}

pub(crate) fn receipt_codec() -> impl Options {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(MAX_PROOF as u64)
        .reject_trailing_bytes()
}

const PREPARATION_V2: Digest = Digest([
    0xd7, 0xcf, 0x4f, 0x9e, 0xef, 0x17, 0x69, 0xa3, 0xfe, 0x1d, 0x2c, 0x70, 0x42, 0x7e, 0x05, 0x31,
    0x3d, 0xf2, 0xa3, 0xef, 0x05, 0x61, 0xd6, 0x48, 0x57, 0xe0, 0xab, 0x51, 0xbc, 0x6f, 0xb9, 0x87,
]);

const HISTORY_QUERY_V2: Digest = Digest([
    0x7d, 0x9e, 0x26, 0x04, 0xef, 0xfe, 0xf9, 0xa1, 0xcb, 0xb5, 0x43, 0x5a, 0x2c, 0xda, 0x96, 0x8f,
    0xd6, 0x9c, 0x32, 0x6a, 0x75, 0x85, 0x95, 0xcb, 0x43, 0x5b, 0x8e, 0x78, 0x24, 0x4b, 0x01, 0xf1,
]);

const POOL_V2: Digest = Digest([
    0xfd, 0xbb, 0xb8, 0x9a, 0xfa, 0x6b, 0x18, 0xa7, 0x5b, 0x6d, 0xab, 0x74, 0x00, 0xde, 0xe0, 0x8f,
    0x4f, 0x10, 0x66, 0x84, 0x79, 0x3e, 0x43, 0x25, 0xac, 0x58, 0xa6, 0xc8, 0xe4, 0x8b, 0x9f, 0xaf,
]);

const SOURCE_V1: Digest = Digest([
    0xc5, 0xa1, 0xd8, 0x68, 0xe1, 0x76, 0x64, 0x79, 0x66, 0xce, 0x6e, 0x72, 0x88, 0x60, 0xc9, 0x22,
    0x5c, 0x99, 0xe0, 0x5a, 0x0d, 0x88, 0x80, 0xa4, 0x74, 0xe1, 0xae, 0xeb, 0x65, 0x2e, 0xc6, 0x69,
]);

const POOL_V1: Digest = Digest([
    0x33, 0xe5, 0xd5, 0x57, 0xb9, 0xab, 0x66, 0x53, 0x70, 0x85, 0x93, 0x81, 0xd0, 0x80, 0x85, 0xe5,
    0xdb, 0xb7, 0x90, 0xef, 0xd8, 0x1b, 0x22, 0x04, 0x9e, 0x57, 0x38, 0xe0, 0x36, 0xc4, 0xb6, 0x02,
]);

const SOURCE_UPDATE_V1: Digest = Digest([
    0x6c, 0x90, 0x33, 0x60, 0x5a, 0x58, 0xfa, 0xe8, 0xcb, 0x4a, 0x1d, 0x97, 0x3e, 0x81, 0x92, 0x3f,
    0xec, 0x8d, 0xf4, 0x6b, 0x8d, 0xa5, 0xdf, 0xf8, 0x44, 0x5a, 0x69, 0xe5, 0x22, 0x85, 0x03, 0x8a,
]);

const SOURCE_V2: Digest = Digest([
    0xeb, 0x46, 0xba, 0x69, 0xfa, 0x4f, 0x33, 0x33, 0x6c, 0x5e, 0x6a, 0x4c, 0xa3, 0x55, 0x96, 0x14,
    0xc8, 0x4c, 0xe8, 0x4d, 0xcb, 0xfe, 0x32, 0x57, 0x70, 0x0d, 0x54, 0xf6, 0xfa, 0x47, 0x42, 0x15,
]);
const SOURCE_UPDATE_V2: Digest = Digest([
    0x26, 0x79, 0x7b, 0xf4, 0x0b, 0xd9, 0x93, 0x49, 0xe3, 0x30, 0x61, 0xbd, 0x58, 0x26, 0xbd, 0xf8,
    0xdc, 0x05, 0xa5, 0x20, 0x0b, 0x08, 0x5f, 0x1e, 0x85, 0x9c, 0x33, 0x28, 0x1c, 0xc4, 0x07, 0x10,
]);

const SOURCE_V3: Digest = Digest([
    0xea, 0x94, 0x53, 0x1a, 0x1a, 0x48, 0xfa, 0xe9, 0x7e, 0x86, 0x0b, 0xd4, 0x16, 0xd5, 0x32, 0x1e,
    0x9d, 0xa0, 0xbf, 0x38, 0x6e, 0x20, 0x7d, 0x8a, 0xc9, 0x2f, 0xfb, 0xc0, 0x04, 0x97, 0xa6, 0xac,
]);
const SOURCE_UPDATE_V3: Digest = Digest([
    0x77, 0x42, 0xc9, 0x55, 0xd7, 0x24, 0x28, 0xf0, 0xc6, 0x43, 0xa7, 0xd9, 0x2c, 0xd4, 0xaa, 0x81,
    0x56, 0x77, 0x79, 0xa4, 0x6c, 0x48, 0xd2, 0xe6, 0xe2, 0x4c, 0x2c, 0x7c, 0x2a, 0x73, 0x66, 0x7b,
]);

const SOURCE_V4: Digest = Digest([
    0x8a, 0x14, 0x4e, 0xde, 0xb2, 0xd5, 0x2a, 0x0c, 0xbc, 0x5a, 0xce, 0xb1, 0xaf, 0x94, 0x25, 0xf9,
    0x05, 0x84, 0x17, 0x6f, 0xc4, 0x1d, 0x41, 0xaa, 0xf7, 0x05, 0x41, 0xf1, 0x9a, 0xee, 0xd3, 0x39,
]);
const SOURCE_UPDATE_V4: Digest = Digest([
    0xf5, 0xb1, 0x4f, 0x1f, 0xa2, 0x5a, 0x45, 0xa6, 0xae, 0x86, 0x31, 0x60, 0x06, 0xe8, 0x02, 0xd7,
    0xad, 0xcf, 0x1d, 0xf0, 0x87, 0xff, 0x8f, 0x47, 0x79, 0xda, 0x01, 0x92, 0x2c, 0x1d, 0xc3, 0x7f,
]);

const SOURCE_V5: Digest = Digest([
    0x55, 0xff, 0x5c, 0x6b, 0x46, 0xec, 0x01, 0x18, 0xd3, 0x72, 0xa7, 0xd2, 0x73, 0x9d, 0xe2, 0x0c,
    0x61, 0xf0, 0x43, 0x87, 0xf5, 0x52, 0x23, 0xf8, 0x7b, 0x3f, 0x53, 0x5a, 0x89, 0xc6, 0xf5, 0x1c,
]);

pub(crate) fn admitted_range_program(image: Digest) -> bool {
    image == crate::source::source_image_id() || image == SOURCE_V5
}

pub(crate) fn admitted_source_pair(source: Digest, update: Digest) -> bool {
    (source == crate::source::source_image_id() && update == crate::source::update_image_id())
        || (source == SOURCE_V1 && update == SOURCE_UPDATE_V1)
        || (source == SOURCE_V2 && update == SOURCE_UPDATE_V2)
        || (source == SOURCE_V3 && update == SOURCE_UPDATE_V3)
        || (source == SOURCE_V4 && update == SOURCE_UPDATE_V4)
        || (source == SOURCE_V5 && update == SOURCE_UPDATE_V4)
}

pub(crate) fn source_height_supported(source: Digest, genesis: Digest, height: u32) -> bool {
    use zcash_protocol::consensus::NetworkType;
    use zutti_source::bootstrap::genesis_hash;
    if genesis_hash(NetworkType::Main).is_ok_and(|expected| genesis == expected) {
        return true;
    }
    if genesis_hash(NetworkType::Test).is_ok_and(|expected| genesis == expected) {
        return !matches!(source, SOURCE_V1 | SOURCE_V2)
            || height < zutti_source::upgrade::TESTNET_NU7_HEIGHT;
    }
    false
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod source_upgrade_tests {
    use super::*;
    use zcash_protocol::consensus::NetworkType;
    use zutti_source::{bootstrap::genesis_hash, upgrade::TESTNET_NU7_HEIGHT};

    #[test]
    fn legacy_source_programs_cannot_admit_nu7_testnet_history() {
        let test = genesis_hash(NetworkType::Test).unwrap();
        let main = genesis_hash(NetworkType::Main).unwrap();
        for program in [SOURCE_V1, SOURCE_V2] {
            assert!(source_height_supported(
                program,
                test,
                TESTNET_NU7_HEIGHT - 1
            ));
            assert!(!source_height_supported(program, test, TESTNET_NU7_HEIGHT));
            assert!(!source_height_supported(program, test, u32::MAX));
            assert!(source_height_supported(program, main, TESTNET_NU7_HEIGHT));
        }
        assert!(source_height_supported(
            crate::source::source_image_id(),
            test,
            TESTNET_NU7_HEIGHT
        ));
        assert!(!source_height_supported(
            crate::source::source_image_id(),
            Digest([1; 32]),
            0
        ));
    }
}
