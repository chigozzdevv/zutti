use crate::{Error, Result};
use core::fmt;
#[cfg(target_os = "zkvm")]
use risc0_zkvm::sha::{Impl, Sha256};
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

#[cfg(not(target_os = "zkvm"))]
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

#[cfg(target_os = "zkvm")]
pub fn hash(tag: &[u8], parts: &[&[u8]]) -> Digest {
    Digest((*Impl::hash_bytes(&hash_preimage(tag, parts))).into())
}

#[cfg(any(target_os = "zkvm", test))]
fn hash_preimage(tag: &[u8], parts: &[&[u8]]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(
        b"ZUTTI\0SHA256\x01".len()
            + 16
            + tag.len()
            + parts.iter().map(|part| 8 + part.len()).sum::<usize>(),
    );
    bytes.extend_from_slice(b"ZUTTI\0SHA256\x01");
    bytes.extend_from_slice(&(tag.len() as u64).to_le_bytes());
    bytes.extend_from_slice(tag);
    bytes.extend_from_slice(&(parts.len() as u64).to_le_bytes());
    for part in parts {
        bytes.extend_from_slice(&(part.len() as u64).to_le_bytes());
        bytes.extend_from_slice(part);
    }
    bytes
}

pub fn commit(tag: &[u8], domain: Digest, payload: &[u8], randomness: &Secret32) -> Digest {
    hash(tag, &[&domain.0, payload, randomness.expose()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contiguous_preimages_preserve_streaming_hashes_and_part_boundaries() {
        for tag in [b"".as_slice(), b"state-branch", b"source-policy"] {
            assert_eq!(
                hash(tag, &[]).0,
                Sha256::digest(hash_preimage(tag, &[])).as_slice()
            );
            for length in [0, 1, 31, 55, 56, 63, 64, 127, 128, 255, 256, 257, 511, 4096] {
                let payload = vec![0x5a; length];
                for parts in [
                    vec![payload.as_slice()],
                    vec![b"".as_slice(), payload.as_slice(), b"".as_slice()],
                    vec![b"left".as_slice(), payload.as_slice(), b"right".as_slice()],
                ] {
                    let expected: [u8; 32] = Sha256::digest(hash_preimage(tag, &parts)).into();
                    assert_eq!(hash(tag, &parts).0, expected);
                }
            }
        }
        assert_ne!(hash(b"tag", &[b"ab", b"c"]), hash(b"tag", &[b"a", b"bc"]));
        assert_ne!(hash(b"tag", &[]), hash(b"tag", &[b""]));
    }
}
