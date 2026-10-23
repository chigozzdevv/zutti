use super::{Error, Result};
use zcash_primitives::block::BlockHeader;

pub const HEADER_BYTES: usize = 1487;

pub fn parse_header(bytes: &[u8]) -> Result<BlockHeader> {
    if bytes.len() != HEADER_BYTES || bytes[140..143] != [0xfd, 0x40, 0x05] {
        return Err(Error::Bounds);
    }
    let mut remaining = bytes;
    let header = BlockHeader::read(&mut remaining).map_err(|_| crate::Error::Block)?;
    if !remaining.is_empty() || header.version < 4 || header.solution.len() != 1344 {
        return Err(crate::Error::Block.into());
    }
    Ok(header)
}
