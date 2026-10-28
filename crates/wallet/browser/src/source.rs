use super::{BrowserWallet, Result, clock, decode, profile, source_receipt};
use orchard::keys::FullViewingKey;
use risc0_zkvm::VerifierContext;
use serde::Deserialize;
use serde_json::json;
use wasm_bindgen::prelude::*;
use zeroize::Zeroizing;
use zutti_protocol::{
    Digest,
    amount::{Amount, Asset, FeeBps},
    encoding::Canonical,
    private_note::NoteRecipient,
};
use zutti_source::spv::HeaderClaim;
use zutti_wallet::{
    order::{MakerRecipients, OrderContext, PreparedOrder, QuoteTerms},
    source_notes::{SourceScan, scan_ironwood},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScanRequest {
    receipt: String,
    previous_height: u32,
    previous_hash: String,
    tree: String,
    blocks: Vec<String>,
    anchor: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Recipient {
    owner: String,
    viewing: String,
    nullifier_key_commitment: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OrderRequest {
    scan: String,
    receipt: String,
    offer_id: String,
    amount: String,
    gross_usdc: String,
    fee_bps: u16,
    fee_caps: [String; 3],
    nonce: String,
    deadline: u64,
    maker_address: String,
    refund: Recipient,
}

fn digest(text: &str) -> Result<Digest> {
    Digest::from_hex(text.strip_prefix("0x").unwrap_or(text)).map_err(|_| "invalid digest".into())
}

fn head(encoded: &str) -> Result<HeaderClaim> {
    let (receipt, head) = source_receipt(&decode(encoded, super::MAX_RECEIPT)?)?;
    receipt
        .verify_with_context(
            &VerifierContext::default().with_dev_mode(false),
            profile::profile()?.policy.program().0,
        )
        .map_err(|_| "source proof verification failed")?;
    Ok(head)
}

#[wasm_bindgen]
impl BrowserWallet {
    pub fn scan_source(&self, request: &str) -> Result<String> {
        if request.len() > super::MAX_REQUEST {
            return Err("scan request exceeds its bound".into());
        }
        let request: ScanRequest =
            serde_json::from_str(request).map_err(|_| "invalid scan request")?;
        let head = head(&request.receipt)?;
        if request.blocks.is_empty()
            || request.blocks.len() > 4096
            || request
                .previous_height
                .checked_add(request.blocks.len() as u32)
                != Some(head.height().map_err(|_| "source height")?)
        {
            return Err("scan range does not match the verified source".into());
        }
        let blocks = request
            .blocks
            .iter()
            .map(|block| decode(block, 2 * 1024 * 1024))
            .collect::<Result<Vec<_>>>()?;
        let scan = scan_ironwood(
            &FullViewingKey::from(&self.source),
            request.previous_height,
            digest(&request.previous_hash)?,
            &decode(&request.tree, 4096)?,
            &blocks,
            decode(&request.anchor, 32)?
                .try_into()
                .map_err(|_| "source anchor length")?,
        )
        .map_err(|_| "source note scan failed")?;
        if scan.block != head.tip().map_err(|_| "source tip")? {
            return Err("scan is outside the verified source history".into());
        }
        let units = scan
            .notes
            .iter()
            .try_fold(0_u128, |sum, coin| {
                sum.checked_add(u128::from(coin.note.value().inner()))
            })
            .ok_or("source balance overflow")?;
        let state = Zeroizing::new(scan.to_bytes().map_err(|_| "scan encoding")?);
        Ok(json!({ "height": scan.height, "block": scan.block.to_string(),
            "balance": Amount::from_units(Asset::Zec, units).map_err(|_| "source balance")?.to_string(),
            "state": self.seal_state(b"source-scan", &state)?,
        }).to_string())
    }

    pub fn prepare_order(&self, request: &str, now: f64) -> Result<String> {
        if request.len() > super::MAX_REQUEST {
            return Err("order request exceeds its bound".into());
        }
        let request: OrderRequest =
            serde_json::from_str(request).map_err(|_| "invalid order request")?;
        if request.deadline <= clock(now)?.checked_mul(1000).ok_or("wallet clock")? {
            return Err("quote has expired".into());
        }
        let profile = profile::profile()?;
        let scan = SourceScan::from_bytes(&self.open_state(b"source-scan", &request.scan)?)
            .map_err(|_| "invalid saved scan")?;
        let anchor = head(&request.receipt)?;
        let refund = NoteRecipient::new(
            decode(&request.refund.owner, 33)?
                .try_into()
                .map_err(|_| "refund owner")?,
            decode(&request.refund.viewing, 32)?
                .try_into()
                .map_err(|_| "refund viewing key")?,
            digest(&request.refund.nullifier_key_commitment)?,
        )
        .map_err(|_| "invalid refund recipient")?;
        let order = PreparedOrder::prepare(
            OrderContext {
                domain: profile.domain,
                policy: profile.policy,
                anchor,
                source: &scan,
                viewing: &FullViewingKey::from(&self.source),
                payout: self.notes.recipient().map_err(|_| "payout recipient")?,
            },
            QuoteTerms {
                offer: digest(&request.offer_id)?,
                zec: Amount::parse(Asset::Zec, &request.amount).map_err(|_| "ZEC amount")?,
                gross_usdc: Amount::parse(Asset::Usdc, &request.gross_usdc)
                    .map_err(|_| "USDC amount")?,
                fee_bps: FeeBps::new(request.fee_bps).map_err(|_| "maker fee")?,
                fee_caps: [
                    Amount::parse(Asset::Zec, &request.fee_caps[0]).map_err(|_| "ZEC fee cap")?,
                    Amount::parse(Asset::Eth, &request.fee_caps[1]).map_err(|_| "ETH fee cap")?,
                    Amount::parse(Asset::Usdc, &request.fee_caps[2]).map_err(|_| "USDC fee cap")?,
                ],
                nonce: digest(&request.nonce)?,
            },
            MakerRecipients::from_address(&request.maker_address, refund)
                .map_err(|_| "maker recipient")?,
        )
        .map_err(|error| format!("payment preparation failed: {error}"))?;
        self.order_result(order, false)
    }

    pub fn prove_order(&self, state: &str) -> Result<String> {
        let order = PreparedOrder::from_bytes(&self.open_state(b"prepared-order", state)?)
            .map_err(|_| "invalid prepared order")?;
        let reservation = order.reservation();
        let proven = zutti_wallet::source_notes::prove_payment(
            order.payment(),
            &reservation.terms,
            &reservation.order_randomness,
        )
        .map_err(|_| "native Zcash transaction proving failed")?;
        self.order_result(
            order
                .with_payment(proven)
                .map_err(|_| "proven payment mismatch")?,
            true,
        )
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use incrementalmerkletree::{Hashable, Level};
    use orchard::{
        Note,
        keys::Scope,
        note::{NoteVersion, RandomSeed, Rho},
        tree::{MerkleHashOrchard, MerklePath},
        value::NoteValue,
    };
    use zutti_wallet::source_notes::SourceNote;

    #[test]
    fn prepared_payment_keeps_source_keys_inside_authenticated_state() {
        let wallet = BrowserWallet::new(vec![42; 32]).unwrap();
        let viewing = FullViewingKey::from(&wallet.source);
        let receipt = hex::encode(include_bytes!(
            "../../../../deploy/ethereum/optimized/source-receipt.bin"
        ));
        let head = head(&receipt).unwrap();
        let height = head.height().unwrap();
        let rho = Rho::from_bytes(&[1; 32]).into_option().unwrap();
        let seed = (0..=255)
            .find_map(|n| RandomSeed::from_bytes([n; 32], &rho).into_option())
            .unwrap();
        let note = Note::from_parts(
            viewing.address_at(0_u32, Scope::External),
            NoteValue::from_raw(500_010_000),
            rho,
            seed,
            NoteVersion::V3,
        )
        .into_option()
        .unwrap();
        let path = MerklePath::from_parts(
            0,
            std::array::from_fn(|i| MerkleHashOrchard::empty_root(Level::from(i as u8))),
        );
        let anchor = path.root(note.commitment().into()).to_bytes();
        let scan = SourceScan {
            height,
            block: head.tip().unwrap(),
            anchor,
            notes: vec![SourceNote {
                note,
                path,
                txid: Digest([1; 32]),
                action: 0,
                received_height: height,
            }],
        };
        let recipient = wallet.notes.recipient().unwrap();
        let now = 1_793_200_000_u64;
        let request = json!({ "scan": wallet.seal_state(b"source-scan", &scan.to_bytes().unwrap()).unwrap(), "receipt": receipt,
            "offer_id": "11".repeat(32), "amount": "0.01", "gross_usdc": "1", "fee_bps": 50,
            "fee_caps": ["0.001", "0.005", "0"], "nonce": "22".repeat(32), "deadline": now * 1000 + 15000,
            "maker_address": wallet.account().unwrap().parse::<serde_json::Value>().unwrap()["zcash_address"],
            "refund": { "owner": hex::encode(recipient.owner()), "viewing": hex::encode(recipient.viewing()),
                "nullifier_key_commitment": recipient.nullifier_key().to_string() },
        });
        let result: serde_json::Value = serde_json::from_str(
            &wallet
                .prepare_order(&request.to_string(), now as f64)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(result["zec"], "0.01");
        assert_eq!(result["usdc"], "0.995");
        assert_eq!(result["native_proof"], false);
        assert!(
            !result
                .to_string()
                .contains(&hex::encode(viewing.to_bytes()))
        );
        let encrypted = result["state"].as_str().unwrap();
        let prepared =
            PreparedOrder::from_bytes(&wallet.open_state(b"prepared-order", encrypted).unwrap())
                .unwrap();
        assert_eq!(
            hex::encode(
                prepared
                    .reservation()
                    .txid
                    .0
                    .iter()
                    .rev()
                    .copied()
                    .collect::<Vec<_>>()
            ),
            result["txid"]
        );
        if let Ok(path) = std::env::var("ZUTTI_BROWSER_TEST_FIXTURE") {
            std::fs::write(path, serde_json::to_vec(&result).unwrap()).unwrap();
        }
        assert!(
            wallet
                .prepare_order(&request.to_string(), (now + 20) as f64)
                .is_err()
        );
        assert!(
            BrowserWallet::new(vec![43; 32])
                .unwrap()
                .open_state(b"prepared-order", encrypted)
                .is_err()
        );
    }

    #[test]
    fn wasm_proven_payment_extracts_with_a_valid_native_proof() {
        let Ok(path) = std::env::var("ZUTTI_BROWSER_PROVEN_FIXTURE") else {
            return;
        };
        let wallet = BrowserWallet::new(vec![42; 32]).unwrap();
        let packet: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let state = wallet
            .open_state(b"prepared-order", packet["state"].as_str().unwrap())
            .unwrap();
        let order = PreparedOrder::from_bytes(&state).unwrap();
        let pczt = pczt::Pczt::parse(order.payment()).unwrap();
        assert!(pczt.ironwood().zkproof().is_some());
        let mut signer = pczt::roles::signer::Signer::new(pczt).unwrap();
        let mut rng = rand::rngs::StdRng::from_seed([7; 32]);
        use rand::SeedableRng;
        signer
            .sign_ironwood(
                &mut rng,
                0,
                &orchard::keys::SpendAuthorizingKey::from(&wallet.source),
            )
            .unwrap();
        let transaction = pczt::roles::tx_extractor::TransactionExtractor::new(signer.finish())
            .extract(&mut rng)
            .unwrap();
        assert_eq!(transaction.txid().as_ref(), &order.reservation().txid.0);
    }
}

impl BrowserWallet {
    fn order_result(&self, order: PreparedOrder, proven: bool) -> Result<String> {
        let reservation = order.reservation();
        let plaintext = Zeroizing::new(order.to_bytes().map_err(|_| "order encoding")?);
        Ok(json!({
            "state": self.seal_state(b"prepared-order", &plaintext)?,
            "offer_id": reservation.terms.offer().to_string(),
            "txid": hex::encode(reservation.txid.0.iter().rev().copied().collect::<Vec<_>>()),
            "zec": reservation.terms.zec().to_string(), "usdc": reservation.terms.usdc().to_string(),
            "recipient": super::address(*reservation.terms.receiver())?,
            "expiry_height": reservation.terms.window().expiry(), "native_proof": proven,
            "reservation": hex::encode(reservation.to_bytes().map_err(|_| "reservation encoding")?),
            "binding_witness": hex::encode(order.binding_witness().map_err(|_| "binding witness")?.to_bytes().map_err(|_| "binding encoding")?),
        }).to_string())
    }
}
