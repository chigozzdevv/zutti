use crate::{Error, Result, block::ParsedBlock};
use zcash_protocol::consensus::NetworkType;
use zutti_protocol::Digest;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target([u8; 32]);
impl Target {
    pub fn from_compact(bits: u32) -> Result<Self> {
        if bits & 0x0080_0000 != 0 {
            return Err(Error::Work);
        }
        let exponent = bits >> 24;
        let mantissa = bits & 0x007f_ffff;
        let mut target = [0; 32];
        if exponent <= 3 {
            let value = mantissa >> (8 * (3 - exponent));
            target[..4].copy_from_slice(&value.to_le_bytes());
        } else {
            let offset = (exponent - 3) as usize;
            for (i, byte) in mantissa.to_le_bytes()[..3].iter().enumerate() {
                if *byte != 0 {
                    *target.get_mut(offset + i).ok_or(Error::Work)? = *byte;
                }
            }
        }
        if target == [0; 32] {
            return Err(Error::Work);
        }
        Ok(Self(target))
    }
    pub const fn little_endian(self) -> [u8; 32] {
        self.0
    }
    pub fn accepts(self, hash: Digest) -> bool {
        hash.0.iter().rev().cmp(self.0.iter().rev()).is_le()
    }
    fn within_limit(self, network: NetworkType) -> Result<bool> {
        let mut limit = [255; 32];
        match network {
            NetworkType::Main => {
                limit[31] = 0;
                limit[30] = 7;
            }
            NetworkType::Test => limit[31] = 7,
            NetworkType::Regtest => return Err(Error::Network),
        }
        Ok(self.0.iter().rev().cmp(limit.iter().rev()).is_le())
    }
}

#[derive(Debug)]
pub struct CheckedWork {
    block: Digest,
    target: Target,
}
impl CheckedWork {
    pub const fn block(&self) -> Digest {
        self.block
    }
    pub const fn target(&self) -> Target {
        self.target
    }
}

pub fn verify_work(block: &ParsedBlock, network: NetworkType) -> Result<CheckedWork> {
    let header = block.header();
    let target = Target::from_compact(header.bits)?;
    if header.version < 4 || !target.within_limit(network)? || !target.accepts(block.hash()) {
        return Err(Error::Work);
    }
    if header.solution.len() != 1344 {
        return Err(Error::Equihash);
    }
    let mut encoded = Vec::new();
    header.write(&mut encoded).map_err(|_| Error::Block)?;
    let input = encoded.get(..108).ok_or(Error::Block)?;
    zcash_primitives::block::equihash::is_valid_solution(
        200,
        9,
        input,
        &header.nonce,
        &header.solution,
    )
    .map_err(|_| Error::Equihash)?;
    Ok(CheckedWork {
        block: block.hash(),
        target,
    })
}
