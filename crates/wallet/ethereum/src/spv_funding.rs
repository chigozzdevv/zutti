use crate::{
    inventory::{self, InventoryEvidence, InventoryRequest},
    spv_authorization::{Error, Result, SpvAuthorizationProfile},
};
use alloy::primitives::{Address, B256};
use risc0_zkvm::{InnerReceipt, Receipt, VerifierContext};
use zutti_protocol::{Digest, encoding::Canonical};
use zutti_wallet::{
    funding::{FundingConsent, SpvFundingPlan},
    private_note::NoteKeys,
    spv_pool::SpvPoolWitness,
};

pub struct SpvFundingProfile<'a> {
    pub swap: &'a SpvAuthorizationProfile,
    pub registry: Address,
    pub registry_code_hash: B256,
}

pub struct FundingEvidence<'a> {
    pub ethereum: &'a InventoryEvidence,
    pub binding: &'a Receipt,
    pub anchor: &'a Receipt,
}

pub struct NoteFundingEvidence<'a> {
    pub ethereum: &'a crate::notes::NoteEvidence,
    pub binding: &'a Receipt,
    pub anchor: &'a Receipt,
    pub headers: &'a [Vec<u8>],
}

impl SpvFundingProfile<'_> {
    pub fn authorize_from_note(
        &self,
        plan: SpvFundingPlan,
        consent: &FundingConsent<'_>,
        keys: &NoteKeys,
        evidence: NoteFundingEvidence<'_>,
        now: u64,
    ) -> Result<SpvPoolWitness> {
        plan.check(consent, keys).map_err(|_| Error::Profile)?;
        let request = self.request(&plan, keys)?;
        if request.nullifiers.len() != 1
            || !matches!(
                evidence.binding.inner,
                InnerReceipt::Succinct(_) | InnerReceipt::Groth16(_)
            )
        {
            return Err(Error::Proof);
        }
        verify_receipt(
            self.swap.binding_program,
            &plan
                .reservation()
                .binding
                .to_bytes()
                .map_err(|_| Error::Profile)?,
            evidence.binding,
        )?;
        verify_receipt(
            self.swap.policy.program(),
            &plan
                .reservation()
                .anchor
                .to_bytes()
                .map_err(|_| Error::Profile)?,
            evidence.anchor,
        )?;
        let current =
            crate::spv_authorization::extend_source(&plan.reservation().anchor, evidence.headers)?;
        plan.reservation()
            .terms
            .window()
            .require_remaining(
                current.height().map_err(|_| Error::Source)?,
                self.swap.minimum_blocks,
            )
            .map_err(|_| Error::Source)?;
        let timestamp = u64::from(current.timestamp().map_err(|_| Error::Source)?);
        if timestamp > now.saturating_add(self.swap.future_drift)
            || u64::from(current.maximum_time()) > now.saturating_add(self.swap.future_drift)
            || now > timestamp.saturating_add(self.swap.maximum_source_age)
        {
            return Err(Error::Source);
        }
        crate::notes::verify(
            &self.swap.ethereum,
            &crate::notes::NoteRequest {
                chain_id: request.chain_id,
                pool: request.pool,
                pool_code_hash: request.pool_code_hash,
                root: request.root,
                nullifier: request.nullifiers[0],
            },
            evidence.ethereum,
            now,
        )?;
        plan.authorize(consent, keys).map_err(|_| Error::Profile)
    }
    pub fn request(&self, plan: &SpvFundingPlan, keys: &NoteKeys) -> Result<InventoryRequest> {
        let swap = self.swap;
        let claim = plan.claim();
        if swap.domain.id().map_err(|_| Error::Profile)? != claim.pool.domain
            || swap.domain.source_policy() != swap.policy.id().map_err(|_| Error::Profile)?
            || swap.domain.payment_profile() != zutti_wallet::binding::payment_profile()
            || swap.ethereum.chain_id != swap.domain.destination_chain()
            || swap.policy != plan.reservation().anchor.policy()
            || claim.binding_program != swap.binding_program
            || claim.pool_program != swap.pool_program
            || self.registry != swap.source
            || self.registry_code_hash != swap.source_code_hash
            || self.registry.is_zero()
            || self.registry_code_hash.is_zero()
            || swap.minimum_blocks == 0
            || swap.minimum_blocks > swap.policy.window_blocks()
        {
            return Err(Error::Profile);
        }
        Ok(InventoryRequest {
            chain_id: swap.ethereum.chain_id,
            pool: Address::from(swap.domain.pool()),
            pool_code_hash: swap.pool_code_hash,
            source: self.registry,
            source_code_hash: self.registry_code_hash,
            root: B256::from(claim.pool.root.0),
            checkpoint: B256::from(claim.pool.checkpoint.0),
            order_tag: B256::from(claim.pool.order_tag.0),
            nullifiers: plan
                .nullifiers(keys)
                .map_err(|_| Error::Profile)?
                .into_iter()
                .map(|n| B256::from(n.0))
                .collect(),
            expiry: claim.expiry,
            confirmations: swap.policy.confirmations(),
            minimum_remaining: swap.minimum_blocks,
            maximum_source_age: swap.maximum_source_age,
            future_drift: swap.future_drift,
        })
    }

    pub fn authorize(
        &self,
        plan: SpvFundingPlan,
        consent: &FundingConsent<'_>,
        keys: &NoteKeys,
        evidence: FundingEvidence<'_>,
        now: u64,
    ) -> Result<SpvPoolWitness> {
        plan.check(consent, keys).map_err(|_| Error::Profile)?;
        let request = self.request(&plan, keys)?;
        if !matches!(
            evidence.binding.inner,
            InnerReceipt::Succinct(_) | InnerReceipt::Groth16(_)
        ) {
            return Err(Error::Proof);
        }
        for (program, journal, receipt) in [
            (
                self.swap.binding_program,
                plan.reservation()
                    .binding
                    .to_bytes()
                    .map_err(|_| Error::Profile)?,
                evidence.binding,
            ),
            (
                self.swap.policy.program(),
                plan.reservation()
                    .anchor
                    .to_bytes()
                    .map_err(|_| Error::Profile)?,
                evidence.anchor,
            ),
        ] {
            verify_receipt(program, &journal, receipt)?;
        }
        inventory::verify(&self.swap.ethereum, &request, evidence.ethereum, now)?;
        plan.authorize(consent, keys).map_err(|_| Error::Profile)
    }
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

#[cfg(test)]
#[path = "spv_funding_tests.rs"]
mod tests;
