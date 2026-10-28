use crate::{ConsensusConfig, Evidence, ReservationRequest, VerifiedReservation};
use alloy::primitives::{Address, B256};
#[cfg(feature = "spv-fulfillment")]
use pczt::roles::signer::SpendAuthSignature;
use risc0_zkvm::{
    Groth16Receipt, Groth16ReceiptVerifierParameters, InnerReceipt, MaybePruned, Receipt,
    ReceiptClaim, VerifierContext, sha::Digestible,
};
use zutti_protocol::{
    Digest, domain::Domain, encoding::Canonical, order::OrderTerms, private_note::NoteRecipient,
};
use zutti_source::spv::{HeaderClaim, Policy};
#[cfg(feature = "spv-fulfillment")]
use zutti_wallet::payment::SpvPaymentJournal;
use zutti_wallet::spv_pool::recovery::SpvSwapRecord;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("SPV authorization profile or saved payment does not match")]
    Profile,
    #[error("SPV authorization requires genuine matching proof evidence")]
    Proof,
    #[error("SPV payment evidence is stale, outside the window or on another ancestry")]
    Source,
    #[error(transparent)]
    Ethereum(#[from] crate::Error),
    #[error(transparent)]
    Wallet(#[from] zutti_wallet::Error),
}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone)]
pub struct SpvAuthorizationProfile {
    pub domain: Domain,
    pub ethereum: ConsensusConfig,
    pub pool_code_hash: B256,
    pub source: Address,
    pub source_code_hash: B256,
    pub binding_program: Digest,
    pub pool_program: Digest,
    pub policy: Policy,
    pub minimum_blocks: u32,
    pub maximum_source_age: u64,
    pub future_drift: u64,
}
pub struct SpvPaymentConsent<'a> {
    pub terms: &'a OrderTerms,
    pub receiving: NoteRecipient,
}
pub struct SpvSourceEvidence<'a> {
    pub receipt: &'a Receipt,
    pub head: &'a HeaderClaim,
    pub headers_from_anchor: &'a [Vec<u8>],
    pub continuation: &'a [Vec<u8>],
}
#[cfg(feature = "spv-fulfillment")]
pub struct AuthorizedSpvPayment<'a> {
    journal: &'a SpvPaymentJournal,
    verified: VerifiedSpvPayment,
}
pub struct VerifiedSpvPayment {
    domain: Digest,
    swap: Digest,
    txid: Digest,
    reservation: VerifiedReservation,
    window: zutti_protocol::policy::PaymentWindow,
    checked_at: u64,
    source_height: u32,
    source_time: u64,
    minimum_blocks: u32,
    maximum_source_age: u64,
    maximum_ethereum_age: u64,
    future_drift: u64,
}
impl SpvAuthorizationProfile {
    pub fn reservation_request(&self, record: &SpvSwapRecord) -> Result<ReservationRequest> {
        record.to_bytes().map_err(|_| Error::Profile)?;
        if record.terms().domain() != &self.domain
            || self.domain.payment_profile() != zutti_wallet::binding::payment_profile()
            || self.ethereum.chain_id != self.domain.destination_chain()
            || self.pool_code_hash.is_zero()
            || self.source.is_zero()
            || self.source_code_hash.is_zero()
            || self.binding_program == Digest::default()
            || self.pool_program == Digest::default()
            || record.funding().pool_program != self.pool_program
            || record.funding().binding_program != self.binding_program
            || self.policy.id().map_err(|_| Error::Profile)? != self.domain.source_policy()
            || self.policy != record.reservation().anchor.policy()
            || self.minimum_blocks == 0
            || self.minimum_blocks > self.policy.window_blocks()
            || self.maximum_source_age == 0
            || self.maximum_source_age > 86_400
            || self.future_drift > 7200
        {
            return Err(Error::Profile);
        }
        Ok(ReservationRequest {
            source: Some(crate::SourceGuard {
                address: self.source,
                code_hash: self.source_code_hash,
                checkpoint: B256::from(record.funding().pool.checkpoint.0),
            }),
            chain_id: self.domain.destination_chain(),
            pool: Address::from(self.domain.pool()),
            pool_code_hash: self.pool_code_hash,
            escrow: B256::from(
                record
                    .reservation()
                    .escrow
                    .commitment()
                    .map_err(|_| Error::Profile)?
                    .0,
            ),
            order_tag: B256::from(record.funding().pool.order_tag.0),
            nullifier: B256::from(
                record
                    .reservation()
                    .escrow
                    .nullifier()
                    .map_err(|_| Error::Profile)?
                    .0,
            ),
        })
    }
    #[cfg(feature = "spv-fulfillment")]
    pub fn verify<'a>(
        &self,
        journal: &'a SpvPaymentJournal,
        consent: SpvPaymentConsent<'_>,
        ethereum: &Evidence,
        source: SpvSourceEvidence<'_>,
        now: u64,
    ) -> Result<AuthorizedSpvPayment<'a>> {
        journal.state()?;
        let verified = self.verify_record(
            journal.record(),
            journal.unsigned_pczt(),
            consent,
            ethereum,
            source,
            now,
        )?;
        Ok(AuthorizedSpvPayment { journal, verified })
    }
    pub fn verify_record(
        &self,
        record: &SpvSwapRecord,
        pczt: &[u8],
        consent: SpvPaymentConsent<'_>,
        ethereum: &Evidence,
        source: SpvSourceEvidence<'_>,
        now: u64,
    ) -> Result<VerifiedSpvPayment> {
        let request = self.reservation_request(record)?;
        verify_consent(record, &consent)?;
        let payment = record.inspect_payment(pczt)?;
        retained_funding(record)?;
        verify_receipt(
            self.policy.program(),
            &source.head.to_bytes().map_err(|_| Error::Source)?,
            source.receipt,
        )?;
        if source.head.policy() != self.policy {
            return Err(Error::Source);
        }
        let proved_height = source.head.height().map_err(|_| Error::Source)?;
        let anchor_height = record
            .reservation()
            .anchor
            .height()
            .map_err(|_| Error::Source)?;
        if proved_height
            .checked_sub(anchor_height)
            .map(|distance| distance as usize)
            != Some(source.headers_from_anchor.len())
        {
            return Err(Error::Source);
        }
        let expected = if source.headers_from_anchor.is_empty() {
            record.reservation().anchor.clone()
        } else {
            record
                .reservation()
                .anchor
                .advance(source.headers_from_anchor)
                .map_err(|_| Error::Source)?
        };
        if expected.id().map_err(|_| Error::Source)?
            != source.head.id().map_err(|_| Error::Source)?
        {
            return Err(Error::Source);
        }
        let current = extend_source(source.head, source.continuation)?;
        let height = current.height().map_err(|_| Error::Source)?;
        record
            .terms()
            .window()
            .require_remaining(height, self.minimum_blocks)
            .map_err(|_| Error::Source)?;
        let time = u64::from(current.timestamp().map_err(|_| Error::Source)?);
        source_time(time, self.maximum_source_age, self.future_drift, now)?;
        if u64::from(current.maximum_time()) > now.saturating_add(self.future_drift) {
            return Err(Error::Source);
        }
        let reservation = crate::verify(&self.ethereum, &request, ethereum, now)?;
        Ok(VerifiedSpvPayment {
            domain: record.domain(),
            swap: record.id().map_err(|_| Error::Profile)?,
            txid: payment.txid(),
            reservation,
            window: record.terms().window(),
            checked_at: now,
            source_height: height,
            source_time: time,
            minimum_blocks: self.minimum_blocks,
            maximum_source_age: self.maximum_source_age,
            maximum_ethereum_age: self.ethereum.max_finalized_age,
            future_drift: self.future_drift,
        })
    }
}

pub(crate) fn extend_source(head: &HeaderClaim, headers: &[Vec<u8>]) -> Result<HeaderClaim> {
    if headers.len() > head.policy().window_blocks() as usize {
        return Err(Error::Source);
    }
    if headers.is_empty() {
        Ok(head.clone())
    } else {
        head.advance(headers).map_err(|_| Error::Source)
    }
}
fn verify_consent(record: &SpvSwapRecord, consent: &SpvPaymentConsent<'_>) -> Result<()> {
    if consent.terms != record.terms()
        || consent.receiving != record.reservation().payout.note.recipient()
    {
        return Err(Error::Profile);
    }
    Ok(())
}
fn verify_funding(program: Digest, record: &SpvSwapRecord, receipt: &Receipt) -> Result<()> {
    if !matches!(
        receipt.inner,
        InnerReceipt::Succinct(_) | InnerReceipt::Groth16(_)
    ) {
        return Err(Error::Proof);
    }
    verify_receipt(
        program,
        &record.funding().to_bytes().map_err(|_| Error::Proof)?,
        receipt,
    )
}
fn verify_receipt(program: Digest, journal: &[u8], receipt: &Receipt) -> Result<()> {
    receipt
        .verify_with_context(&VerifierContext::default().with_dev_mode(false), program.0)
        .map_err(|_| Error::Proof)?;
    if receipt.journal.bytes != journal {
        return Err(Error::Proof);
    }
    Ok(())
}
fn source_time(timestamp: u64, maximum_age: u64, future_drift: u64, now: u64) -> Result<()> {
    if timestamp > now.saturating_add(future_drift) || now > timestamp.saturating_add(maximum_age) {
        return Err(Error::Source);
    }
    Ok(())
}
impl VerifiedSpvPayment {
    pub const fn domain(&self) -> Digest {
        self.domain
    }
    pub const fn swap(&self) -> Digest {
        self.swap
    }
    pub const fn txid(&self) -> Digest {
        self.txid
    }
    pub fn fresh(&self, now: u64) -> Result<()> {
        if now < self.checked_at
            || now
                > self
                    .reservation
                    .execution_timestamp
                    .saturating_add(self.maximum_ethereum_age)
        {
            return Err(Error::Source);
        }
        source_time(
            self.source_time,
            self.maximum_source_age,
            self.future_drift,
            now,
        )?;
        self.window
            .require_remaining(self.source_height, self.minimum_blocks)
            .map_err(|_| Error::Source)
    }
}
#[cfg(feature = "spv-fulfillment")]
impl AuthorizedSpvPayment<'_> {
    fn fresh(&self, now: u64) -> Result<()> {
        self.verified.fresh(now)
    }
    pub fn sign<F>(&self, now: u64, signer: F) -> Result<()>
    where
        F: FnOnce(&[u8]) -> zutti_wallet::Result<Vec<SpendAuthSignature>>,
    {
        self.fresh(now)?;
        self.journal.begin_signing()?;
        self.journal
            .record_signatures(&signer(self.journal.unsigned_pczt())?)?;
        Ok(())
    }
    pub fn reconcile_signatures(&self, now: u64, signatures: &[SpendAuthSignature]) -> Result<()> {
        self.fresh(now)?;
        self.journal.record_signatures(signatures)?;
        Ok(())
    }
    pub fn signing_request(&self, now: u64) -> Result<&[u8]> {
        self.fresh(now)?;
        match self.journal.state()? {
            zutti_wallet::payment::PaymentState::Prepared => self.journal.begin_signing()?,
            zutti_wallet::payment::PaymentState::Signing => {}
            _ => return Err(Error::Wallet(zutti_wallet::Error::PaymentState)),
        }
        Ok(self.journal.unsigned_pczt())
    }
    pub fn reconcile_pczt(&self, now: u64, signed: &[u8]) -> Result<()> {
        self.fresh(now)?;
        self.journal.record_signed_pczt(signed)?;
        Ok(())
    }
    pub fn finalize(&self) -> Result<Digest> {
        Ok(self.journal.finalize()?)
    }
    pub fn finalize_with_proof(&self, pczt: Vec<u8>) -> Result<Digest> {
        Ok(self.journal.finalize_with_proof(pczt)?)
    }
    pub fn submit<F>(&self, now: u64, transport: F) -> Result<Digest>
    where
        F: FnOnce(&[u8]) -> zutti_wallet::Result<Digest>,
    {
        self.fresh(now)?;
        Ok(self.journal.submit(transport)?)
    }
}

#[cfg(all(test, feature = "spv-fulfillment"))]
#[path = "spv_authorization_tests.rs"]
mod tests;

fn retained_funding(record: &SpvSwapRecord) -> Result<Receipt> {
    recover_funding(record, record.funding_seal().ok_or(Error::Proof)?)
}

pub fn recover_funding(record: &SpvSwapRecord, seal: &[u8]) -> Result<Receipt> {
    let journal = record.funding().to_bytes().map_err(|_| Error::Proof)?;
    let receipt = ethereum_receipt(record.funding().pool_program, &journal, seal)?;
    verify_funding(record.funding().pool_program, record, &receipt)?;
    Ok(receipt)
}
fn ethereum_receipt(program: Digest, journal: &[u8], seal: &[u8]) -> Result<Receipt> {
    let parameters = Groth16ReceiptVerifierParameters::default().digest();
    if seal.len() != 260 || seal[..4] != parameters.as_bytes()[..4] {
        return Err(Error::Proof);
    }
    let receipt = Receipt::new(
        InnerReceipt::Groth16(Groth16Receipt::new(
            seal[4..].to_vec(),
            MaybePruned::Value(ReceiptClaim::ok(program.0, journal.to_vec())),
            parameters,
        )),
        journal.to_vec(),
    );
    verify_receipt(program, journal, &receipt)?;
    Ok(receipt)
}
