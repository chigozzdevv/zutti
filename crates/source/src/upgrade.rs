use crate::{Error, Result};
use zcash_protocol::consensus::{BlockHeight, BranchId, MAIN_NETWORK, NetworkType, TEST_NETWORK};

pub const TESTNET_NU7_HEIGHT: u32 = 4_465_026;

pub fn consensus_branch(network: NetworkType, height: u32) -> Result<BranchId> {
    let height = BlockHeight::from_u32(height);
    match network {
        NetworkType::Main => Ok(BranchId::for_height(&MAIN_NETWORK, height)),
        NetworkType::Test => Ok(BranchId::for_height(&TEST_NETWORK, height)),
        NetworkType::Regtest => Err(Error::Network),
    }
}
