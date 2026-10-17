use crate::{Error, Result};
use core::fmt;
#[cfg(target_os = "zkvm")]
use risc0_zkvm::sha::rust_crypto::{Digest as _, Sha256};
#[cfg(not(target_os = "zkvm"))]
use sha2::{Digest as _, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Digest(pub [u8; 32]);

impl Digest {
    pub fn nonzero(bytes: [u8; 32]) -> Result<Self> {
        if bytes == [0; 32] {
            Err(Error::Invalid("zero identifier"))
        } else {
            Ok(Self(bytes))
        }
    }
    pub fn from_hex(text: &str) -> Result<Self> {
        if text.len() != 64 {
            return Err(Error::Invalid("digest length"));
        }
        let mut bytes = [0; 32];
        hex::decode_to_slice(text, &mut bytes).map_err(|_| Error::Invalid("digest hex"))?;
        Ok(Self(bytes))
    }
}
impl fmt::Debug for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Digest({})", hex::encode(self.0))
    }
}
impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(self.0))
    }
}

#[derive(Zeroize, ZeroizeOnDrop)]
pub struct Secret32([u8; 32]);
impl Secret32 {
    pub fn from_bytes(bytes: [u8; 32]) -> Result<Self> {
        if bytes == [0; 32] {
            Err(Error::Invalid("zero secret"))
        } else {
            Ok(Self(bytes))
        }
    }
    #[cfg(feature = "entropy")]
    pub fn random() -> Result<Self> {
        let mut bytes = [0; 32];
        getrandom::fill(&mut bytes).map_err(|_| Error::Entropy)?;
        Self::from_bytes(bytes)
    }
    pub fn expose(&self) -> &[u8; 32] {
        &self.0
    }
}
impl fmt::Debug for Secret32 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret32([REDACTED])")
    }
}

pub fn hash(tag: &[u8], parts: &[&[u8]]) -> Digest {
    let mut state = Sha256::new();
    state.update(b"ZUTTI\0SHA256\x01");
    state.update((tag.len() as u64).to_le_bytes());
    state.update(tag);
    state.update((parts.len() as u64).to_le_bytes());
    for part in parts {
        state.update((part.len() as u64).to_le_bytes());
        state.update(part);
    }
    Digest(state.finalize().into())
}

pub fn commit(tag: &[u8], domain: Digest, payload: &[u8], randomness: &Secret32) -> Digest {
    hash(tag, &[&domain.0, payload, randomness.expose()])
}
