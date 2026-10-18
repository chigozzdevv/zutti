use crate::{Error, Result};
use zcash_protocol::consensus::NetworkType;

pub const TESTNET_NU7_HEIGHT: u32 = 4_465_026;

pub fn ensure_supported(network: NetworkType, height: u32) -> Result<()> {
    match network {
        NetworkType::Main => Ok(()),
        NetworkType::Test if height < TESTNET_NU7_HEIGHT => Ok(()),
        NetworkType::Test => Err(Error::Upgrade),
        NetworkType::Regtest => Err(Error::Network),
    }
}
