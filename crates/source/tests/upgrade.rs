#![cfg(feature = "blocks")]
use zcash_protocol::consensus::NetworkType;
use zutti_source::{
    Error,
    upgrade::{TESTNET_NU7_HEIGHT, ensure_supported},
};
#[test]
fn the_source_profile_stops_at_the_first_unsupported_testnet_activation() {
    assert!(ensure_supported(NetworkType::Test, 0).is_ok());
    assert!(ensure_supported(NetworkType::Test, TESTNET_NU7_HEIGHT - 1).is_ok());
    assert_eq!(
        ensure_supported(NetworkType::Test, TESTNET_NU7_HEIGHT),
        Err(Error::Upgrade)
    );
    assert_eq!(
        ensure_supported(NetworkType::Test, u32::MAX),
        Err(Error::Upgrade)
    );
    assert!(ensure_supported(NetworkType::Main, TESTNET_NU7_HEIGHT).is_ok());
    assert_eq!(
        ensure_supported(NetworkType::Regtest, 0),
        Err(Error::Network)
    );
}
