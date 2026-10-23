use super::{Error, Result};
use crate::block::ParsedBlock;
use zcash_protocol::consensus::{MAIN_NETWORK, NetworkType, TEST_NETWORK};

pub fn parse_block(bytes: &[u8], network: NetworkType) -> Result<ParsedBlock> {
    let block = match network {
        NetworkType::Main => ParsedBlock::parse(bytes, &MAIN_NETWORK)?,
        NetworkType::Test => ParsedBlock::parse(bytes, &TEST_NETWORK)?,
        _ => return Err(Error::Policy),
    };
    for transaction in block.transactions() {
        if transaction.version().header() & 0x7fff_ffff < 5 {
            let mut encoded = Vec::new();
            transaction
                .write(&mut encoded)
                .map_err(|_| crate::Error::Block)?;
            if encoded.len() == 64 {
                return Err(Error::AmbiguousMerkleLeaf);
            }
        }
    }
    Ok(block)
}
