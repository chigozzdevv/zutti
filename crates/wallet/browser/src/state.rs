use super::{BrowserWallet, Result, decode, profile};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use rand::{TryRng, rngs::SysRng};
use zeroize::Zeroizing;

const MAX_STATE: usize = 16 * 1024 * 1024;

impl BrowserWallet {
    fn state_aad(&self, kind: &[u8]) -> Result<Vec<u8>> {
        let mut aad = b"zutti-browser-state-v1".to_vec();
        aad.extend_from_slice(&profile::profile()?.domain.id().map_err(|_| "domain")?.0);
        aad.extend_from_slice(kind);
        Ok(aad)
    }

    pub(super) fn seal_state(&self, kind: &[u8], plaintext: &[u8]) -> Result<String> {
        if plaintext.is_empty() || plaintext.len() > MAX_STATE {
            return Err("wallet state exceeds its bound".into());
        }
        let mut nonce = [0; 24];
        SysRng
            .try_fill_bytes(&mut nonce)
            .map_err(|_| "secure randomness unavailable")?;
        let aad = self.state_aad(kind)?;
        let cipher =
            XChaCha20Poly1305::new_from_slice(self.state.expose()).map_err(|_| "state key")?;
        let ciphertext = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .map_err(|_| "wallet state encryption failed")?;
        let mut bytes = nonce.to_vec();
        bytes.extend_from_slice(&ciphertext);
        Ok(hex::encode(bytes))
    }

    pub(super) fn open_state(&self, kind: &[u8], encoded: &str) -> Result<Zeroizing<Vec<u8>>> {
        let bytes = decode(encoded, MAX_STATE + 40)?;
        if bytes.len() <= 40 {
            return Err("invalid encrypted wallet state".into());
        }
        let aad = self.state_aad(kind)?;
        let cipher =
            XChaCha20Poly1305::new_from_slice(self.state.expose()).map_err(|_| "state key")?;
        cipher
            .decrypt(
                XNonce::from_slice(&bytes[..24]),
                Payload {
                    msg: &bytes[24..],
                    aad: &aad,
                },
            )
            .map(Zeroizing::new)
            .map_err(|_| "wallet state authentication failed".into())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn state_is_authenticated_to_wallet_purpose_and_deployment() {
        let wallet = BrowserWallet::new(vec![42; 32]).unwrap();
        let encrypted = wallet
            .seal_state(b"order", b"private preparation data")
            .unwrap();
        assert_eq!(
            wallet.open_state(b"order", &encrypted).unwrap().as_slice(),
            b"private preparation data"
        );
        assert!(wallet.open_state(b"scan", &encrypted).is_err());
        assert!(
            BrowserWallet::new(vec![43; 32])
                .unwrap()
                .open_state(b"order", &encrypted)
                .is_err()
        );
        let mut changed = hex::decode(encrypted).unwrap();
        changed[24] ^= 1;
        assert!(wallet.open_state(b"order", &hex::encode(changed)).is_err());
    }
}
