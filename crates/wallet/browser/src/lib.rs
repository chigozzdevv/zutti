mod deposit;
mod profile;

#[cfg(test)]
#[path = "../../../../tests/support/payment.rs"]
mod payment_fixture;
#[cfg(test)]
#[path = "../../../../tests/support/mod.rs"]
mod support;

use bincode::Options;
use hkdf::Hkdf;
use orchard::{
    keys::{FullViewingKey, PreparedIncomingViewingKey, Scope, SpendAuthorizingKey, SpendingKey},
    note_encryption::OrchardDomain,
};
use pczt::{Pczt, roles::signer::Signer};
use rand::{
    SeedableRng,
    rngs::{StdRng, SysRng},
};
use risc0_zkvm::{Receipt, VerifierContext};
use serde::Deserialize;
use serde_json::json;
use sha2::Sha256;
use wasm_bindgen::prelude::*;
use zcash_address::unified::{Address, Encoding, Receiver, Uitem};
use zcash_protocol::{address::Revision, consensus::NetworkType};
use zeroize::Zeroizing;
use zutti_ethereum::{
    Evidence,
    spv_authorization::{SpvPaymentConsent, SpvSourceEvidence, VerifiedSpvPayment},
};
use zutti_protocol::{Secret32, encoding::Canonical, private_note::NoteRecipient};
use zutti_source::spv::{HEADER_BYTES, HeaderClaim, MAX_HEADERS};
use zutti_wallet::{prepare::MAX_PCZT, private_note::NoteKeys, spv_pool::recovery::SpvSwapRecord};

const MAX_REQUEST: usize = 48 * 1024 * 1024;
const MAX_RECEIPT: usize = 16 * 1024 * 1024;

type Result<T> = std::result::Result<T, String>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SigningRequest {
    record: String,
    pczt: String,
    ethereum: Evidence,
    source: SourceEvidence,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceEvidence {
    receipt: String,
    headers_from_anchor: Vec<String>,
    continuation: Vec<String>,
}

#[wasm_bindgen]
pub struct BrowserWallet {
    source: SpendingKey,
    notes: NoteKeys,
}

#[wasm_bindgen]
pub struct SigningAuthorization {
    payment: VerifiedSpvPayment,
    pczt: Zeroizing<Vec<u8>>,
    owner: NoteRecipient,
    review: String,
    spends: Vec<usize>,
}

fn clock(now: f64) -> Result<u64> {
    if !now.is_finite() || now.fract() != 0.0 || !(1.0..=9_007_199_254_740_991.0).contains(&now) {
        return Err("invalid wallet clock".into());
    }
    Ok(now as u64)
}

fn decode(text: &str, limit: usize) -> Result<Vec<u8>> {
    if text.is_empty() || text.len() > limit * 2 || !text.len().is_multiple_of(2) {
        return Err("invalid or oversized encoded wallet input".into());
    }
    hex::decode(text).map_err(|_| "invalid encoded wallet input".into())
}

fn source_receipt(bytes: &[u8]) -> Result<(Receipt, HeaderClaim)> {
    if bytes.is_empty() || bytes.len() > MAX_RECEIPT {
        return Err("invalid source receipt length".into());
    }
    let receipt: Receipt = bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(MAX_RECEIPT as u64)
        .reject_trailing_bytes()
        .deserialize(bytes)
        .map_err(|_| "invalid source receipt")?;
    let head = HeaderClaim::from_bytes(&receipt.journal.bytes).map_err(|_| "source claim")?;
    if head.policy() != profile::profile()?.policy {
        return Err("source policy mismatch".into());
    }
    Ok((receipt, head))
}

#[wasm_bindgen]
pub fn verify_source(bytes: &[u8]) -> Result<u32> {
    let (receipt, head) = source_receipt(bytes)?;
    let profile = profile::profile()?;
    receipt
        .verify_with_context(
            &VerifierContext::default().with_dev_mode(false),
            profile.policy.program().0,
        )
        .map_err(|_| "source proof verification failed")?;
    head.height().map_err(|_| "source height".into())
}

#[wasm_bindgen]
impl BrowserWallet {
    #[wasm_bindgen(constructor)]
    pub fn new(seed: Vec<u8>) -> Result<Self> {
        let seed = Zeroizing::new(seed);
        if seed.len() != 32 || seed.iter().all(|byte| *byte == 0) {
            return Err("32 bytes of private wallet entropy required".into());
        }
        let source = SpendingKey::from_zip32_seed(&seed, 1, zip32::AccountId::ZERO)
            .map_err(|_| "Zcash key derivation failed")?;
        let hkdf = Hkdf::<Sha256>::new(Some(b"zutti-testnet-note-keys-v1"), &seed);
        let secret = |label: &[u8]| -> Result<Secret32> {
            let mut bytes = Zeroizing::new([0; 32]);
            hkdf.expand(label, bytes.as_mut())
                .map_err(|_| "note key derivation failed")?;
            Secret32::from_bytes(*bytes).map_err(|_| "note key derivation failed".into())
        };
        let notes = NoteKeys::from_secrets(
            secret(b"spending")?,
            secret(b"viewing")?,
            secret(b"nullifier")?,
        )
        .map_err(|_| "note key derivation failed")?;
        Ok(Self { source, notes })
    }

    pub fn account(&self) -> Result<String> {
        let receiver = FullViewingKey::from(&self.source)
            .address_at(0_u32, Scope::External)
            .to_raw_address_bytes();
        let address = address(receiver)?;
        let note = self
            .notes
            .recipient()
            .map_err(|_| "private receiving key")?;
        Ok(json!({
            "version": 1,
            "network": "testnet",
            "chain_id": 11155111,
            "domain": profile::profile()?.domain.id().map_err(|_| "domain")?.to_string(),
            "zcash_address": address,
            "usdc_receiver": {
                "owner": hex::encode(note.owner()),
                "viewing": hex::encode(note.viewing()),
                "nullifier_key_commitment": note.nullifier_key().to_string(),
            },
        })
        .to_string())
    }

    pub fn authorize(&self, request: &str, now: f64) -> Result<SigningAuthorization> {
        let now = clock(now)?;
        if request.is_empty() || request.len() > MAX_REQUEST {
            return Err("signing request exceeds its bound".into());
        }
        let request: SigningRequest =
            serde_json::from_str(request).map_err(|_| "invalid signing request")?;
        let record = SpvSwapRecord::from_bytes(&decode(&request.record, 131_072)?)
            .map_err(|_| "invalid swap record")?;
        let pczt = Zeroizing::new(decode(&request.pczt, MAX_PCZT)?);
        let (receipt, head) = source_receipt(&decode(&request.source.receipt, MAX_RECEIPT)?)?;
        let headers = |values: Vec<String>| -> Result<Vec<Vec<u8>>> {
            if values.len() > MAX_HEADERS {
                return Err("too many source headers".into());
            }
            values
                .into_iter()
                .map(|text| {
                    let bytes = decode(&text, HEADER_BYTES)?;
                    if bytes.len() != HEADER_BYTES {
                        return Err("source header length".into());
                    }
                    Ok(bytes)
                })
                .collect()
        };
        let ancestry = headers(request.source.headers_from_anchor)?;
        let continuation = headers(request.source.continuation)?;
        let owner = self
            .notes
            .recipient()
            .map_err(|_| "private receiving key")?;
        let payment = profile::profile()?
            .verify_record(
                &record,
                &pczt,
                SpvPaymentConsent {
                    terms: record.terms(),
                    receiving: owner,
                },
                &request.ethereum,
                SpvSourceEvidence {
                    receipt: &receipt,
                    head: &head,
                    headers_from_anchor: &ancestry,
                    continuation: &continuation,
                },
                now,
            )
            .map_err(|_| "reservation or payment verification failed")?;
        let (spends, fee) = self.check_ownership(&pczt, record.terms())?;
        Ok(SigningAuthorization {
            review: json!({
                "swap_id": payment.swap().to_string(),
                "txid": payment.txid().to_string(),
                "zec": record.terms().zec().to_string(),
                "usdc": record.terms().usdc().to_string(),
                "fee_zatoshi": fee.to_string(),
                "recipient": address(*record.terms().receiver())?,
                "expiry_height": record.terms().window().expiry(),
            })
            .to_string(),
            payment,
            pczt,
            owner,
            spends,
        })
    }
}

impl BrowserWallet {
    fn check_ownership(
        &self,
        bytes: &[u8],
        terms: &zutti_protocol::order::OrderTerms,
    ) -> Result<(Vec<usize>, u64)> {
        let viewing = FullViewingKey::from(&self.source);
        let incoming = PreparedIncomingViewingKey::new(&viewing.to_ivk(Scope::Internal));
        let mut spends = Vec::new();
        let pczt = Pczt::parse(bytes).map_err(|_| "invalid PCZT")?;
        let fee = pczt.ironwood().value_sum().0;
        pczt::roles::verifier::Verifier::new(pczt)
            .with_ironwood::<(), _>(|bundle| {
                bundle.verify_cross_address_restriction()?;
                for (index, action) in bundle.actions().iter().enumerate() {
                    if action
                        .spend()
                        .value()
                        .is_some_and(|value| value.inner() > 0)
                    {
                        action.spend().verify_nullifier(Some(&viewing))?;
                        action.spend().verify_rk(Some(&viewing))?;
                        spends.push(index);
                    }
                    if action
                        .output()
                        .value()
                        .is_some_and(|value| value.inner() > 0)
                        && !(action
                            .output()
                            .recipient()
                            .map(|r| r.to_raw_address_bytes())
                            .as_ref()
                            == Some(terms.receiver())
                            && action.output().value().map(|v| u128::from(v.inner()))
                                == Some(terms.zec().units()))
                    {
                        let (note, _, _) = zcash_note_encryption::try_note_decryption(
                            &OrchardDomain::for_pczt_action(action),
                            &incoming,
                            action,
                        )
                        .ok_or(pczt::roles::verifier::OrchardError::Custom(()))?;
                        if Some(note.recipient()) != *action.output().recipient()
                            || Some(note.value()) != *action.output().value()
                        {
                            return Err(pczt::roles::verifier::OrchardError::Custom(()));
                        }
                    }
                }
                Ok(())
            })
            .map_err(|_| "payment spends or change do not belong to this wallet")?
            .finish();
        if spends.is_empty() {
            return Err("payment has no owned spend".into());
        }
        Ok((spends, fee))
    }
}

#[wasm_bindgen]
impl SigningAuthorization {
    pub fn review(&self) -> String {
        self.review.clone()
    }

    pub fn sign(self, wallet: &BrowserWallet, now: f64) -> Result<Vec<u8>> {
        self.payment
            .fresh(clock(now)?)
            .map_err(|_| "payment authorization expired; refresh the evidence")?;
        if wallet
            .notes
            .recipient()
            .map_err(|_| "private receiving key")?
            != self.owner
        {
            return Err("payment authorization belongs to another wallet".into());
        }
        let pczt = Pczt::parse(&self.pczt).map_err(|_| "invalid PCZT")?;
        let mut signer = Signer::new(pczt).map_err(|_| "invalid signing request")?;
        let mut rng = StdRng::try_from_rng(&mut SysRng)
            .map_err(|_| "secure signing randomness is unavailable")?;
        for index in self.spends {
            signer
                .sign_ironwood(&mut rng, index, &SpendAuthorizingKey::from(&wallet.source))
                .map_err(|_| "payment signing failed")?;
        }
        signer
            .finish()
            .serialize()
            .map_err(|_| "signed payment encoding failed".into())
    }
}

fn address(receiver: [u8; 43]) -> Result<String> {
    Address::try_from_items(Revision::R0, vec![Uitem::Data(Receiver::Orchard(receiver))])
        .map(|address| address.encode(&NetworkType::Test))
        .map_err(|_| "invalid shielded Testnet receiver".into())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn account_derivation_is_stable_and_network_bound() {
        let a = BrowserWallet::new(vec![42; 32]).unwrap();
        let b = BrowserWallet::new(vec![42; 32]).unwrap();
        let c = BrowserWallet::new(vec![43; 32]).unwrap();
        assert_eq!(a.account().unwrap(), b.account().unwrap());
        assert_ne!(a.account().unwrap(), c.account().unwrap());
        let public: serde_json::Value = serde_json::from_str(&a.account().unwrap()).unwrap();
        assert!(
            public["zcash_address"]
                .as_str()
                .unwrap()
                .starts_with("utest1")
        );
        assert_eq!(public["chain_id"], 11155111);
        assert_eq!(public.as_object().unwrap().len(), 6);
        assert!(BrowserWallet::new(vec![0; 32]).is_err());
        assert!(BrowserWallet::new(vec![1; 31]).is_err());
    }

    #[test]
    fn malformed_or_caller_authorized_requests_cannot_sign() {
        let wallet = BrowserWallet::new(vec![42; 32]).unwrap();
        for request in ["", "{}", "{\"authorized\":true}", "{\"seed\":\"00\"}"] {
            assert!(wallet.authorize(request, 1_793_200_000.0).is_err());
        }
        for now in [f64::NAN, f64::INFINITY, -1.0, 0.0, 1.5, 1e20] {
            assert!(clock(now).is_err());
        }
        assert!(verify_source(&[0; 128]).is_err());
        assert!(decode("00aa", 1).is_err());
    }

    #[test]
    fn payment_ownership_rejects_foreign_spends_and_extra_maker_value() {
        let payment = payment_fixture::fixture(false);
        let pczt = payment.pczt.serialize().unwrap();
        let wallet = BrowserWallet {
            source: payment.key,
            notes: BrowserWallet::new(vec![42; 32]).unwrap().notes,
        };
        let (spends, fee) = wallet.check_ownership(&pczt, &payment.terms).unwrap();
        assert_eq!(spends, vec![payment.spend]);
        assert_eq!(fee, 10_000);
        assert!(
            BrowserWallet::new(vec![43; 32])
                .unwrap()
                .check_ownership(&pczt, &payment.terms)
                .is_err()
        );
        let different = payment_fixture::fixture_custom(false, "0.001", 4_465_101, |input| {
            input.zec =
                zutti_protocol::amount::Amount::parse(zutti_protocol::amount::Asset::Zec, "4")
                    .unwrap();
        });
        assert!(wallet.check_ownership(&pczt, &different.terms).is_err());
    }
}
