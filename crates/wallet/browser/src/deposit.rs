use super::{BrowserWallet, Result, decode, profile};
use bincode::Options;
use rand::{TryRng, rngs::SysRng};
use risc0_zkvm::{Receipt, VerifierContext, sha::Digestible};
use serde_json::json;
use wasm_bindgen::prelude::*;
use zeroize::Zeroizing;
use zutti_protocol::{
    Secret32,
    amount::{Amount, Asset},
    encoding::Canonical,
    pool::OperationKind,
};
use zutti_wallet::{
    deposit::DepositContext,
    private_note::NoteEncryption,
    spv_pool::{MAX_SPV_POOL_WITNESS, SpvAction, SpvPoolWitness},
};

#[wasm_bindgen]
impl BrowserWallet {
    pub fn prepare_deposit(&self, amount: &str, owner: &str) -> Result<String> {
        let owner: [u8; 20] = decode(owner.strip_prefix("0x").ok_or("deposit owner")?, 20)?
            .try_into()
            .map_err(|_| "deposit owner")?;
        let profile = profile::profile()?;
        let context = DepositContext::new(
            profile.domain.clone(),
            profile.policy,
            profile.binding_program,
            profile.pool_program,
            owner,
        )
        .map_err(|_| "deposit context")?;
        let amount = Amount::parse(Asset::Usdc, amount).map_err(|_| "invalid USDC amount")?;
        let mut entropy = Zeroizing::new([0; 120]);
        SysRng
            .try_fill_bytes(entropy.as_mut())
            .map_err(|_| "secure randomness unavailable")?;
        let secret = |start: usize| -> Result<Secret32> {
            Secret32::from_bytes(
                entropy[start..start + 32]
                    .try_into()
                    .map_err(|_| "randomness length")?,
            )
            .map_err(|_| "deposit randomness".into())
        };
        let witness = context
            .prepare_for(
                self.notes.recipient().map_err(|_| "deposit recipient")?,
                amount,
                secret(0).map_err(|_| "deposit randomness")?,
                secret(32).map_err(|_| "deposit randomness")?,
                NoteEncryption::from_entropy(
                    secret(64).map_err(|_| "encryption randomness")?,
                    entropy[96..120]
                        .try_into()
                        .map_err(|_| "encryption nonce")?,
                ),
            )
            .map_err(|_| "deposit preparation failed")?;
        let output = &witness.outputs[0];
        Ok(json!({
            "version": 1, "chain_id": 11155111,
            "domain": witness.claim.pool.domain.to_string(),
            "pool": format!("0x{}", hex::encode(profile.domain.pool())),
            "token": format!("0x{}", hex::encode(profile.domain.token())),
            "owner": format!("0x{}", hex::encode(owner)),
            "amount": amount.to_string(), "units": amount.units().to_string(),
            "commitment": output.note.commitment().map_err(|_| "deposit commitment")?.to_string(),
            "witness": hex::encode(witness.to_bytes().map_err(|_| "deposit witness")?),
        })
        .to_string())
    }

    pub fn verify_deposit(&self, witness: &str, proof: &str) -> Result<String> {
        let witness = SpvPoolWitness::from_bytes(&decode(witness, MAX_SPV_POOL_WITNESS)?)
            .map_err(|_| "invalid deposit witness")?;
        let profile = profile::profile()?;
        let claim = witness
            .evaluate()
            .map_err(|_| "invalid deposit operation")?;
        if !matches!(witness.action, SpvAction::Deposit)
            || claim.pool.kind != OperationKind::Deposit
            || claim.pool.domain != profile.domain.id().map_err(|_| "domain")?
            || claim.policy != profile.policy.id().map_err(|_| "policy")?
            || claim.binding_program != profile.binding_program
            || claim.pool_program != profile.pool_program
            || witness.outputs.len() != 1
            || !witness.inputs.is_empty()
            || witness.outputs[0].note.recipient()
                != self.notes.recipient().map_err(|_| "recipient")?
        {
            return Err("deposit does not belong to this wallet and deployment".into());
        }
        let bytes = decode(proof, super::MAX_RECEIPT)?;
        let receipt: Receipt = bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .with_limit(super::MAX_RECEIPT as u64)
            .reject_trailing_bytes()
            .deserialize(&bytes)
            .map_err(|_| "invalid deposit receipt")?;
        let journal = claim.to_bytes().map_err(|_| "deposit journal")?;
        if receipt.journal.bytes != journal {
            return Err("deposit proof journal mismatch".into());
        }
        receipt
            .verify_with_context(
                &VerifierContext::default().with_dev_mode(false),
                profile.pool_program.0,
            )
            .map_err(|_| "deposit proof verification failed")?;
        let risc0_zkvm::InnerReceipt::Groth16(inner) = &receipt.inner else {
            return Err("Ethereum-compatible deposit proof required".into());
        };
        let parameters = risc0_zkvm::Groth16ReceiptVerifierParameters::default().digest();
        if inner.verifier_parameters != parameters || inner.seal.len() != 256 {
            return Err("unsupported deposit verifier".into());
        }
        let mut seal = parameters.as_bytes()[..4].to_vec();
        seal.extend_from_slice(&inner.seal);
        let ciphertext = witness.outputs[0]
            .encrypted()
            .map_err(|_| "deposit encryption")?;
        let commitment = witness.outputs[0]
            .note
            .commitment()
            .map_err(|_| "deposit commitment")?;
        let calldata = calldata(
            &seal,
            claim.pool.value,
            claim.pool.owner,
            commitment.0,
            &ciphertext.to_bytes().map_err(|_| "deposit ciphertext")?,
        );
        Ok(json!({
            "chain_id": 11155111, "to": format!("0x{}", hex::encode(profile.domain.pool())),
            "token": format!("0x{}", hex::encode(profile.domain.token())),
            "from": format!("0x{}", hex::encode(claim.pool.owner)),
            "units": claim.pool.value.to_string(), "value": "0x0", "data": format!("0x{}", hex::encode(calldata)),
        }).to_string())
    }
}

fn word(value: u64) -> [u8; 32] {
    let mut result = [0; 32];
    result[24..].copy_from_slice(&value.to_be_bytes());
    result
}

fn dynamic(bytes: &[u8]) -> Vec<u8> {
    let mut result = word(bytes.len() as u64).to_vec();
    result.extend_from_slice(bytes);
    result.resize(32 + bytes.len().div_ceil(32) * 32, 0);
    result
}

fn calldata(
    seal: &[u8],
    value: u64,
    owner: [u8; 20],
    commitment: [u8; 32],
    ciphertext: &[u8],
) -> Vec<u8> {
    let proof = dynamic(seal);
    let mut result = vec![0x1d, 0x0a, 0x9b, 0x2e];
    result.extend_from_slice(&word(64));
    result.extend_from_slice(&word(64 + proof.len() as u64));
    result.extend_from_slice(&proof);
    result.extend_from_slice(&word(1));
    result.extend_from_slice(&[0; 32 * 6]);
    result.extend_from_slice(&word(value));
    result.extend_from_slice(&[0; 12]);
    result.extend_from_slice(&owner);
    result.extend_from_slice(&word(352));
    result.extend_from_slice(&word(384));
    result.extend_from_slice(&word(0));
    result.extend_from_slice(&word(1));
    result.extend_from_slice(&word(32));
    result.extend_from_slice(&commitment);
    result.extend_from_slice(&word(64));
    result.extend_from_slice(&dynamic(ciphertext));
    result
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn deposit_is_owned_randomized_and_pinned() {
        let wallet = BrowserWallet::new(vec![42; 32]).unwrap();
        let owner = format!("0x{}", "22".repeat(20));
        let a: serde_json::Value =
            serde_json::from_str(&wallet.prepare_deposit("1.25", &owner).unwrap()).unwrap();
        let b: serde_json::Value =
            serde_json::from_str(&wallet.prepare_deposit("1.25", &owner).unwrap()).unwrap();
        assert_eq!(a["units"], "1250000");
        assert_eq!(a["owner"], owner);
        assert_ne!(a["commitment"], b["commitment"]);
        let witness =
            SpvPoolWitness::from_bytes(&hex::decode(a["witness"].as_str().unwrap()).unwrap())
                .unwrap();
        let claim = witness.evaluate().unwrap();
        assert_eq!(claim.pool.value, 1_250_000);
        assert_eq!(claim.pool.owner, [0x22; 20]);
        assert!(witness.outputs[0].note.recipient() == wallet.notes.recipient().unwrap());
        assert_eq!(
            wallet
                .notes
                .decrypt(&witness.outputs[0].encrypted().unwrap(), claim.pool.domain)
                .unwrap()
                .amount()
                .units(),
            1_250_000
        );
        assert!(wallet.prepare_deposit("0", &owner).is_err());
        assert!(wallet.prepare_deposit("1.0000001", &owner).is_err());
        assert!(
            wallet
                .prepare_deposit("1", &format!("0x{}", "00".repeat(20)))
                .is_err()
        );
        assert!(
            wallet
                .verify_deposit(a["witness"].as_str().unwrap(), "00")
                .is_err()
        );
        assert!(
            BrowserWallet::new(vec![43; 32])
                .unwrap()
                .verify_deposit(a["witness"].as_str().unwrap(), "00")
                .unwrap_err()
                .contains("belong")
        );
    }

    #[test]
    fn calldata_matches_the_contract_abi() {
        let bytes = calldata(&[0x11; 260], 1_000_000, [0x22; 20], [0x33; 32], &[0x44; 39]);
        assert_eq!(bytes.len(), 996);
        assert_eq!(
            hex::encode(Sha256::digest(bytes)),
            "c2ad6a9b4e28141e588588b08956064a211c69548399bcce50ab8e3d2f37e3b2"
        );
    }
}
