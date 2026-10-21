use crate::{ConsensusConfig, Evidence, ReservationRequest, VerifiedReservation};
use alloy::primitives::{Address, B256};
use pczt::roles::signer::SpendAuthSignature;
use zutti_protocol::{Digest, domain::Domain, encoding::Canonical, statement::ProofKind};
use zutti_prover::{
    admission::{SourceAdmission, VerifiedSource},
    artifact::ProofArtifact,
    program::{Program, ProgramKind},
};
use zutti_source::{
    bootstrap::genesis_hash,
    checkpoint::{HeaderPath, SourceCheckpoint},
    transition::StateSnapshot,
};
use zutti_wallet::{payment::PaymentJournal, preparation::PreparationClaim, swap::SwapRecord};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("authorization profile does not match the saved swap")]
    Profile,
    #[error("preparation receipt does not bind this saved payment")]
    Preparation,
    #[error("source evidence does not authenticate the order's anchor and inclusion window")]
    Source,
    #[error("authorization evidence has expired or the clock moved backwards")]
    Stale,
    #[error(transparent)]
    Ethereum(#[from] crate::Error),
    #[error(transparent)]
    Prover(#[from] zutti_prover::Error),
    #[error(transparent)]
    Wallet(#[from] zutti_wallet::Error),
}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub struct AuthorizationProfile {
    pub domain: Domain,
    pub ethereum: ConsensusConfig,
    pub pool_code_hash: B256,
    pub preparation: Program,
    pub source: SourceAdmission,
    pub minimum_blocks: u32,
}
#[derive(Debug)]
pub struct SourceEvidence<'a> {
    pub proof: &'a ProofArtifact,
    pub state: &'a StateSnapshot,
    pub checkpoint: SourceCheckpoint,
    pub anchor: HeaderPath,
}
#[derive(Debug)]
pub struct AuthorizedPayment<'a> {
    journal: &'a PaymentJournal,
    reservation: VerifiedReservation,
    source: VerifiedSource,
    maximum_ethereum_age: u64,
    minimum_blocks: u32,
    checked_at: u64,
}
impl AuthorizationProfile {
    pub fn reservation_request(&self, record: &SwapRecord) -> Result<ReservationRequest> {
        if record.terms().domain() != &self.domain
            || self.ethereum.chain_id != self.domain.destination_chain()
            || self.pool_code_hash.is_zero()
            || self.preparation.kind() != ProgramKind::Preparation
            || self.source.policy.id().map_err(|_| Error::Profile)? != self.domain.source_policy()
            || genesis_hash(self.source.policy.network).map_err(|_| Error::Profile)?
                != self.domain.source_genesis()
            || self.minimum_blocks == 0
        {
            return Err(Error::Profile);
        }
        Ok(ReservationRequest {
            chain_id: self.domain.destination_chain(),
            pool: Address::from(self.domain.pool()),
            pool_code_hash: self.pool_code_hash,
            escrow: B256::from(record.escrow_commitment()?.0),
            order_tag: B256::from(record.reservation_claim().order_tag.0),
            nullifier: B256::from(record.escrow_nullifier()?.0),
        })
    }
    fn verify_preparation(&self, record: &SwapRecord, artifact: &ProofArtifact) -> Result<()> {
        verify_preparation_binding(
            artifact,
            self.preparation.image(),
            record.reservation_claim(),
        )
    }
    pub fn verify<'a>(
        &self,
        journal: &'a PaymentJournal,
        reservation: &Evidence,
        preparation: &ProofArtifact,
        source: SourceEvidence<'_>,
        now: u64,
    ) -> Result<AuthorizedPayment<'a>> {
        let request = self.reservation_request(journal.record())?;
        journal.state()?;
        journal.record().inspect_payment(journal.unsigned_pczt())?;
        self.verify_preparation(journal.record(), preparation)?;
        let reservation = crate::verify(&self.ethereum, &request, reservation, now)?;
        let verified_source =
            self.source
                .verify(source.proof, source.state, source.checkpoint, now)?;
        verify_source_window(
            &verified_source,
            journal.record().terms().window(),
            &source.anchor,
            self.minimum_blocks,
            now,
        )?;
        Ok(AuthorizedPayment {
            journal,
            reservation,
            source: verified_source,
            maximum_ethereum_age: self.ethereum.max_finalized_age,
            minimum_blocks: self.minimum_blocks,
            checked_at: now,
        })
    }
}
impl AuthorizedPayment<'_> {
    fn fresh(&self, now: u64) -> Result<()> {
        verify_time(
            self.checked_at,
            self.reservation.execution_timestamp,
            self.maximum_ethereum_age,
            now,
        )?;
        self.source.checkpoint(now)?;
        self.journal
            .record()
            .terms()
            .window()
            .require_remaining(self.source.height(), self.minimum_blocks)
            .map_err(|_| Error::Source)
    }
    pub fn sign<F>(&self, now: u64, signer: F) -> Result<()>
    where
        F: FnOnce(&[u8]) -> zutti_wallet::Result<Vec<SpendAuthSignature>>,
    {
        self.fresh(now)?;
        self.journal.begin_signing()?;
        let signatures = signer(self.journal.unsigned_pczt())?;
        self.journal.record_signatures(&signatures)?;
        Ok(())
    }
    pub fn reconcile_signatures(&self, now: u64, signatures: &[SpendAuthSignature]) -> Result<()> {
        self.fresh(now)?;
        self.journal.record_signatures(signatures)?;
        Ok(())
    }
    pub fn finalize(&self) -> Result<Digest> {
        Ok(self.journal.finalize()?)
    }
    pub fn finalize_with_proof(&self, proven_pczt: Vec<u8>) -> Result<Digest> {
        Ok(self.journal.finalize_with_proof(proven_pczt)?)
    }
    pub fn submit<F>(&self, now: u64, transport: F) -> Result<Digest>
    where
        F: FnOnce(&[u8]) -> zutti_wallet::Result<Digest>,
    {
        self.fresh(now)?;
        Ok(self.journal.submit(transport)?)
    }
}

fn verify_preparation_binding(
    artifact: &ProofArtifact,
    program: Digest,
    funding: &zutti_protocol::pool::PoolClaim,
) -> Result<()> {
    let statement = artifact.statement();
    let claim =
        PreparationClaim::from_bytes(statement.payload()).map_err(|_| Error::Preparation)?;
    if statement.kind() != ProofKind::Preparation
        || statement.program() != program
        || statement.domain() != funding.domain
        || claim.domain() != funding.domain
        || claim.order() != funding.order
        || claim.reservation() != funding.reservation
    {
        return Err(Error::Preparation);
    }
    artifact.verify(statement)?;
    Ok(())
}
fn verify_source_window(
    source: &VerifiedSource,
    window: zutti_protocol::policy::PaymentWindow,
    anchor: &HeaderPath,
    minimum_blocks: u32,
    now: u64,
) -> Result<()> {
    let checkpoint = source.checkpoint(now)?;
    if checkpoint.stable.index.context.start() != window.index_start() {
        return Err(Error::Source);
    }
    checkpoint
        .verify_anchor(window.anchor_height(), window.anchor_hash(), anchor)
        .map_err(|_| Error::Source)?;
    window
        .require_remaining(source.height(), minimum_blocks)
        .map_err(|_| Error::Source)
}
fn verify_time(checked_at: u64, execution: u64, maximum_age: u64, now: u64) -> Result<()> {
    if now < checked_at || now < execution || now - execution > maximum_age {
        return Err(Error::Stale);
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../../../tests/support/payment.rs"]
mod payment_fixture;
#[cfg(test)]
#[path = "../../../../tests/support/payment_swap.rs"]
mod payment_swap;
#[cfg(test)]
#[path = "../../../../tests/support/mod.rs"]
mod support;
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use alloy::primitives::hex;
    use bincode::Options;
    use risc0_zkvm::{FakeReceipt, InnerReceipt, Receipt, ReceiptClaim};
    use zcash_protocol::consensus::NetworkType;
    use zutti_protocol::{
        policy::PaymentWindow,
        pool::{OperationKind, PoolClaim, SourceUpdateClaim},
        statement::Statement,
    };
    use zutti_prover::{ProofEnvelope, remote::import_groth16};
    use zutti_source::{bootstrap::GenesisBlock, proof::SourcePolicy, state::SourceCandidate};

    fn receipt(name: &str, kind: ProofKind) -> ProofArtifact {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/fixtures")
            .join(name);
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let payload = hex::decode(value["journal"].as_str().unwrap()).unwrap();
        let domain = match kind {
            ProofKind::Preparation => PreparationClaim::from_bytes(&payload).unwrap().domain(),
            ProofKind::Source => SourceUpdateClaim::from_bytes(&payload).unwrap().policy,
            _ => panic!("unsupported fixture"),
        };
        let statement = Statement::new(
            kind,
            domain,
            Digest::from_hex(value["image_id"].as_str().unwrap()).unwrap(),
            payload,
        )
        .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let private_path = directory.path().join("receipt.json");
        zutti_wallet::backup::persist_new(&private_path, &std::fs::read(path).unwrap()).unwrap();
        import_groth16(&private_path, &statement).unwrap()
    }
    fn preparation_funding(artifact: &ProofArtifact) -> PoolClaim {
        let claim = PreparationClaim::from_bytes(artifact.statement().payload()).unwrap();
        PoolClaim {
            kind: OperationKind::Reserve,
            domain: claim.domain(),
            root: Digest([1; 32]),
            checkpoint: Digest([2; 32]),
            order_tag: Digest([3; 32]),
            order: claim.order(),
            reservation: claim.reservation(),
            value: 0,
            owner: [0; 20],
            nullifiers: Digest([4; 32]),
            outputs: Digest([5; 32]),
        }
    }
    #[test]
    fn genuine_preparation_receipt_requires_the_exact_payment_and_pinned_program() {
        let artifact = receipt("preparation-proof.json", ProofKind::Preparation);
        let funding = preparation_funding(&artifact);
        let program = artifact.statement().program();
        verify_preparation_binding(&artifact, program, &funding).unwrap();
        for field in 0..3 {
            let mut changed = funding.clone();
            match field {
                0 => changed.domain.0[0] ^= 1,
                1 => changed.order.0[0] ^= 1,
                _ => changed.reservation.0[0] ^= 1,
            }
            assert!(verify_preparation_binding(&artifact, program, &changed).is_err());
        }
        assert!(verify_preparation_binding(&artifact, Digest([9; 32]), &funding).is_err());
    }
    #[test]
    fn matching_labels_do_not_admit_a_development_preparation_receipt() {
        let artifact = receipt("preparation-proof.json", ProofKind::Preparation);
        let funding = preparation_funding(&artifact);
        let statement = artifact.statement().clone();
        let receipt = Receipt::new(
            InnerReceipt::Fake(FakeReceipt::new(ReceiptClaim::ok(
                statement.program().0,
                statement.payload().to_vec(),
            ))),
            statement.payload().to_vec(),
        );
        let bytes = bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .serialize(&receipt)
            .unwrap();
        let fake = ProofArtifact::new(
            statement.clone(),
            ProofEnvelope::new(&statement, bytes).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            verify_preparation_binding(&fake, statement.program(), &funding),
            Err(Error::Prover(zutti_prover::Error::DevelopmentReceipt))
        ));
    }
    fn source() -> (VerifiedSource, HeaderPath, u64) {
        let artifact = receipt("source-update-proof.json", ProofKind::Source);
        let claim = SourceUpdateClaim::from_bytes(artifact.statement().payload()).unwrap();
        let policy = SourcePolicy {
            network: NetworkType::Main,
            program: Digest::from_hex(
                "c5a1d868e176647966ce6e728860c9225c99e05a0d8880a474e1aeeb652ec669",
            )
            .unwrap(),
            registry_program: artifact.statement().program(),
            index_start: 0,
            confirmations: 3,
        };
        let admission = SourceAdmission {
            policy,
            minimum_height: 4,
            minimum_work: claim.work_be,
            maximum_age: 600,
            maximum_future: 30,
        };
        let genesis =
            GenesisBlock::parse(zebra_test::vectors::MAINNET_BLOCKS[&0], NetworkType::Main)
                .unwrap();
        let mut candidate = SourceCandidate::from_genesis(&genesis, claim.policy, 0).unwrap();
        candidate
            .record_transition(
                (1..=4)
                    .map(|height| zebra_test::vectors::MAINNET_BLOCKS[&height].to_vec())
                    .collect(),
            )
            .unwrap();
        let state = candidate.state_snapshot().unwrap();
        let now = u64::from(state.timestamp().unwrap()) + 1;
        let verified = admission
            .verify(&artifact, &state, candidate.checkpoint(3).unwrap(), now)
            .unwrap();
        (verified, candidate.prove_header(1).unwrap(), now)
    }
    #[test]
    fn source_gate_checks_ancestry_freshness_and_head_height_before_expiry() {
        let (source, anchor, now) = source();
        let stable = &source.checkpoint(now).unwrap().stable;
        let window = PaymentWindow::new(0, 1, stable.hash, 8).unwrap();
        verify_source_window(&source, window, &anchor, 2, now).unwrap();
        let expiring = PaymentWindow::new(0, 1, stable.hash, 5).unwrap();
        assert!(verify_source_window(&source, expiring, &anchor, 2, now).is_err());
        let wrong_hash = PaymentWindow::new(0, 1, Digest([9; 32]), 8).unwrap();
        assert!(verify_source_window(&source, wrong_hash, &anchor, 2, now).is_err());
        assert!(verify_source_window(&source, window, &anchor, 0, now).is_err());
        assert!(verify_source_window(&source, window, &anchor, 2, now + 600).is_err());
    }
    #[test]
    fn ethereum_freshness_does_not_survive_expiry_or_clock_rollback() {
        verify_time(150, 100, 60, 160).unwrap();
        for now in [99, 149, 161, u64::MAX] {
            assert!(verify_time(150, 100, 60, now).is_err());
        }
    }
    #[test]
    fn reservation_request_is_derived_from_the_saved_swap_and_local_profile() {
        let policy = SourcePolicy {
            network: NetworkType::Test,
            program: Program::current(ProgramKind::Source).image(),
            registry_program: Program::current(ProgramKind::SourceUpdate).image(),
            index_start: 0,
            confirmations: 100,
        };
        let (_, record) = payment_swap::fixture_with_policy(policy.id().unwrap());
        let mut ethereum = crate::tests::fixture("deneb").0;
        ethereum.chain_id = record.terms().domain().destination_chain();
        let mut profile = AuthorizationProfile {
            domain: record.terms().domain().clone(),
            ethereum,
            pool_code_hash: B256::repeat_byte(9),
            preparation: Program::current(ProgramKind::Preparation),
            source: SourceAdmission {
                policy,
                minimum_work: [1; 32],
                minimum_height: 1,
                maximum_age: 600,
                maximum_future: 30,
            },
            minimum_blocks: 10,
        };
        let request = profile.reservation_request(&record).unwrap();
        assert_eq!(request.pool, Address::from(record.terms().domain().pool()));
        assert_eq!(request.escrow.0, record.escrow_commitment().unwrap().0);
        assert_eq!(request.nullifier.0, record.escrow_nullifier().unwrap().0);
        assert_eq!(request.order_tag.0, record.reservation_claim().order_tag.0);
        profile.minimum_blocks = 0;
        assert!(profile.reservation_request(&record).is_err());
        profile.minimum_blocks = 10;
        profile.ethereum.chain_id ^= 1;
        assert!(profile.reservation_request(&record).is_err());
        profile.ethereum.chain_id ^= 1;
        profile.pool_code_hash = B256::ZERO;
        assert!(profile.reservation_request(&record).is_err());
        profile.pool_code_hash = B256::repeat_byte(9);
        profile.source.policy.network = NetworkType::Main;
        assert!(profile.reservation_request(&record).is_err());
        profile.source.policy.network = NetworkType::Test;
        profile.preparation = Program::current(ProgramKind::Pool);
        assert!(profile.reservation_request(&record).is_err());
    }
}
