use crate::{Error, Result};
use orchard::{
    Note,
    note::{NoteVersion, Rho},
    note_encryption::{OrchardDomain, OrchardNoteEncryption},
};
use pczt::Pczt;
use pczt::roles::verifier::{OrchardError, Verifier};
use zcash_note_encryption::Domain as _;
use zcash_primitives::transaction::txid::{TxIdDigester, to_txid};
use zcash_protocol::{
    PoolType,
    consensus::{BlockHeight, BranchId, MAIN_NETWORK, TEST_NETWORK},
};
use zutti_protocol::{Digest, Secret32, order::OrderTerms};

pub const MAX_PCZT: usize = 8 * 1024 * 1024;

fn payment_branch(terms: &OrderTerms) -> Result<BranchId> {
    use zutti_protocol::network::{ZCASH_MAINNET_GENESIS, ZCASH_TESTNET_GENESIS};
    let first = BlockHeight::from_u32(
        terms
            .window()
            .anchor_height()
            .checked_add(1)
            .ok_or(Error::Payment)?,
    );
    let last = BlockHeight::from_u32(terms.window().expiry());
    let (first, last) = match terms.domain().source_genesis() {
        ZCASH_MAINNET_GENESIS => (
            BranchId::for_height(&MAIN_NETWORK, first),
            BranchId::for_height(&MAIN_NETWORK, last),
        ),
        ZCASH_TESTNET_GENESIS => (
            BranchId::for_height(&TEST_NETWORK, first),
            BranchId::for_height(&TEST_NETWORK, last),
        ),
        _ => return Err(Error::Payment),
    };
    if first != last || !matches!(first, BranchId::Nu6_3 | BranchId::Nu7) {
        return Err(Error::Payment);
    }
    Ok(first)
}

pub struct InspectedPayment {
    txid: Digest,
    expiry: u32,
    actions: usize,
}
impl core::fmt::Debug for InspectedPayment {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("InspectedPayment")
            .field("expiry", &self.expiry)
            .field("actions", &self.actions)
            .finish_non_exhaustive()
    }
}
impl InspectedPayment {
    pub const fn txid(&self) -> Digest {
        self.txid
    }
    pub const fn expiry(&self) -> u32 {
        self.expiry
    }
}

fn parse_ironwood(bytes: &[u8], terms: &OrderTerms) -> Result<Pczt> {
    if bytes.is_empty() || bytes.len() > MAX_PCZT {
        return Err(Error::Pczt);
    }
    let pczt = Pczt::parse(bytes).map_err(|_| Error::Pczt)?;
    if *pczt.global().tx_version() != 6
        || *pczt.global().consensus_branch_id() != u32::from(payment_branch(terms)?)
        || *pczt.global().expiry_height() != terms.window().expiry()
        || pczt.has_data_in_pool(PoolType::TRANSPARENT)
        || pczt.has_data_in_pool(PoolType::SAPLING)
        || pczt.has_data_in_pool(PoolType::ORCHARD)
        || !pczt.has_data_in_pool(PoolType::IRONWOOD)
    {
        return Err(Error::Payment);
    }
    if *pczt.ironwood().flags() != 7 {
        return Err(Error::Payment);
    }
    Ok(pczt)
}

pub fn inspect_ironwood(
    bytes: &[u8],
    terms: &OrderTerms,
    order_randomness: &Secret32,
) -> Result<InspectedPayment> {
    let pczt = parse_ironwood(bytes, terms)?;
    let mut rejection = Error::Payment;
    let pczt = Verifier::new(pczt)
        .with_ironwood::<(), _>(|bundle| {
            let mut withheld_real_spend = false;
            let mut spent = 0_u128;
            let mut received = 0_u128;
            for action in bundle.actions() {
                action.verify_cv_net()?;
                action.spend().verify_nullifier(None)?;
                action.spend().verify_rk(None)?;
                action.output().verify_note_commitment(action.spend())?;
                if action.spend().dummy_sk().is_some() {
                    return Err(OrchardError::Custom(()));
                }
                let value = action
                    .spend()
                    .value()
                    .ok_or(OrchardError::Custom(()))?
                    .inner();
                spent = spent
                    .checked_add(u128::from(value))
                    .ok_or(OrchardError::Custom(()))?;
                received = received
                    .checked_add(u128::from(
                        action
                            .output()
                            .value()
                            .ok_or(OrchardError::Custom(()))?
                            .inner(),
                    ))
                    .ok_or(OrchardError::Custom(()))?;
                if value > 0 {
                    if action.spend().spend_auth_sig().is_some() {
                        rejection = Error::AlreadyAuthorized;
                        return Err(OrchardError::Custom(()));
                    }
                    withheld_real_spend = true;
                }
            }
            let fee = spent
                .checked_sub(received)
                .ok_or(OrchardError::Custom(()))?;
            let (magnitude, sign) = bundle.value_sum().magnitude_sign();
            if !withheld_real_spend
                || !matches!(sign, orchard::value::Sign::Positive)
                || u128::from(magnitude) != fee
                || fee > terms.source_fee_cap().units()
            {
                return Err(OrchardError::Custom(()));
            }
            Ok(())
        })
        .map_err(|_| rejection)?
        .finish();
    inspect_bound_output(pczt, terms, order_randomness)
}

pub(crate) fn inspect_payment_effects(
    bytes: &[u8],
    terms: &OrderTerms,
    order_randomness: &Secret32,
) -> Result<InspectedPayment> {
    let pczt = parse_ironwood(bytes, terms)?;
    let (fee, negative) = *pczt.ironwood().value_sum();
    if negative || u128::from(fee) > terms.source_fee_cap().units() {
        return Err(Error::Payment);
    }
    inspect_bound_output(pczt, terms, order_randomness)
}

fn inspect_bound_output(
    pczt: Pczt,
    terms: &OrderTerms,
    order_randomness: &Secret32,
) -> Result<InspectedPayment> {
    let actions = pczt.ironwood().actions().len();
    let memo = terms.memo(terms.commitment(order_randomness)?)?;
    let pczt = Verifier::new(pczt)
        .with_ironwood::<(), _>(|bundle| {
            let mut matching_outputs = 0;
            for action in bundle.actions() {
                let output = action.output();
                if output
                    .recipient()
                    .map(|r| r.to_raw_address_bytes())
                    .as_ref()
                    == Some(terms.receiver())
                    && output.value().map(|v| u128::from(v.inner())) == Some(terms.zec().units())
                {
                    let note = Note::from_parts(
                        output.recipient().ok_or(OrchardError::Custom(()))?,
                        output.value().ok_or(OrchardError::Custom(()))?,
                        Rho::from_bytes(&action.spend().nullifier().to_bytes())
                            .into_option()
                            .ok_or(OrchardError::Custom(()))?,
                        output.rseed().ok_or(OrchardError::Custom(()))?,
                        NoteVersion::V3,
                    )
                    .into_option()
                    .ok_or(OrchardError::Custom(()))?;
                    if orchard::note::ExtractedNoteCommitment::from(note.commitment()).to_bytes()
                        != output.cmx().to_bytes()
                    {
                        return Err(OrchardError::Custom(()));
                    }
                    let encryptor = OrchardNoteEncryption::new(None, note, memo);
                    let ciphertext = output.encrypted_note();
                    if OrchardDomain::epk_bytes(encryptor.epk()).0 != ciphertext.epk_bytes
                        || encryptor.encrypt_note_plaintext().0 != ciphertext.enc_ciphertext.0
                    {
                        return Err(OrchardError::Custom(()));
                    }
                    matching_outputs += 1;
                }
            }
            if matching_outputs != 1 {
                return Err(OrchardError::Custom(()));
            }
            Ok(())
        })
        .map_err(|_| Error::Payment)?
        .finish();
    let effects = pczt.into_effects().map_err(|_| Error::Pczt)?;
    let digest = to_txid(
        effects.version(),
        effects.consensus_branch_id(),
        &effects.digest(TxIdDigester),
    );
    Ok(InspectedPayment {
        txid: Digest(*digest.as_ref()),
        expiry: terms.window().expiry(),
        actions,
    })
}

pub fn authorize(_payment: &InspectedPayment) -> Result<()> {
    Err(Error::AuthorizationUnavailable)
}
