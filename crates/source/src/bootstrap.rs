use crate::{Error, Result, block::ParsedBlock, work::verify_work};
use zcash_protocol::consensus::{MAIN_NETWORK, NetworkType, TEST_NETWORK};
use zutti_protocol::Digest;

pub fn genesis_hash(network: NetworkType) -> Result<Digest> {
    let text = match network {
        NetworkType::Main => "00040fe8ec8471911baa1db1266ea15dd06b4a8a5c453883c000b031973dce08",
        NetworkType::Test => "05a60a92d99d85997cce3b87616c089f6124d7342af37106edc76126334a2c38",
        NetworkType::Regtest => return Err(Error::Network),
    };
    let mut hash = Digest::from_hex(text)?;
    hash.0.reverse();
    Ok(hash)
}

#[derive(Debug)]
pub struct GenesisBlock {
    block: ParsedBlock,
    network: NetworkType,
}
impl GenesisBlock {
    pub fn parse(bytes: &[u8], network: NetworkType) -> Result<Self> {
        let block = match network {
            NetworkType::Main => ParsedBlock::parse(bytes, &MAIN_NETWORK)?,
            NetworkType::Test => ParsedBlock::parse(bytes, &TEST_NETWORK)?,
            NetworkType::Regtest => return Err(Error::Network),
        };
        if block.claimed_height() != 0
            || block.parent() != Digest::default()
            || block.hash() != genesis_hash(network)?
        {
            return Err(Error::Genesis);
        }
        verify_work(&block, network)?;
        Ok(Self { block, network })
    }
    pub const fn block(&self) -> &ParsedBlock {
        &self.block
    }
    pub const fn network(&self) -> NetworkType {
        self.network
    }
}
