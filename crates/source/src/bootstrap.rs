use crate::{Error, Result, block::ParsedBlock, work::verify_work};
use zcash_protocol::consensus::{MAIN_NETWORK, NetworkType, TEST_NETWORK};
use zutti_protocol::Digest;

pub fn genesis_hash(network: NetworkType) -> Result<Digest> {
    match network {
        NetworkType::Main => Ok(zutti_protocol::network::ZCASH_MAINNET_GENESIS),
        NetworkType::Test => Ok(zutti_protocol::network::ZCASH_TESTNET_GENESIS),
        NetworkType::Regtest => Err(Error::Network),
    }
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
