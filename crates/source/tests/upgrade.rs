#![cfg(feature = "blocks")]
use zcash_protocol::consensus::{BlockHeight, BranchId, NetworkType, TEST_NETWORK};
use zutti_source::{
    Error,
    upgrade::{TESTNET_NU7_HEIGHT, consensus_branch},
};
#[test]
fn the_source_profile_uses_the_deployed_nu7_branch_and_activation() {
    assert_eq!(consensus_branch(NetworkType::Test, 0), Ok(BranchId::Sprout));
    assert_eq!(
        consensus_branch(NetworkType::Test, TESTNET_NU7_HEIGHT - 1),
        Ok(BranchId::Nu6_3)
    );
    assert_eq!(
        consensus_branch(NetworkType::Test, TESTNET_NU7_HEIGHT),
        Ok(BranchId::Nu7)
    );
    assert_eq!(
        BranchId::for_height(&TEST_NETWORK, BlockHeight::from_u32(TESTNET_NU7_HEIGHT - 1)),
        BranchId::Nu6_3
    );
    assert_eq!(
        BranchId::for_height(&TEST_NETWORK, BlockHeight::from_u32(TESTNET_NU7_HEIGHT)),
        BranchId::Nu7
    );
    assert_eq!(u32::from(BranchId::Nu7), 0x7719_0ad9);
    assert!(BranchId::try_from(0x7719_0ad8).is_err());
    assert_eq!(
        consensus_branch(NetworkType::Main, TESTNET_NU7_HEIGHT),
        Ok(BranchId::Nu6_3)
    );
    assert_eq!(
        consensus_branch(NetworkType::Regtest, 0),
        Err(Error::Network)
    );
}
