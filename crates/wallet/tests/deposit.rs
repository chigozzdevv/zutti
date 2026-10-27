#![allow(clippy::unwrap_used)]
use zcash_protocol::consensus::NetworkType;
use zutti_protocol::{
    Digest,
    amount::{Amount, Asset},
    domain::Domain,
    encoding::Canonical,
    network::ZCASH_TESTNET_GENESIS,
};
use zutti_source::spv::Policy;
use zutti_wallet::{account::WalletAccount, deposit::DepositContext, spv_pool::SpvPoolWitness};

fn context() -> DepositContext {
    let policy = Policy::new(Digest([1; 32]), NetworkType::Test, 10, 128, 5_000_000).unwrap();
    let domain = Domain::new(
        ZCASH_TESTNET_GENESIS,
        11155111,
        [2; 20],
        [3; 20],
        zutti_wallet::binding::payment_profile(),
        policy.id().unwrap(),
        Digest([4; 32]),
    )
    .unwrap();
    DepositContext::new(domain, policy, Digest([5; 32]), Digest([6; 32]), [7; 20]).unwrap()
}

#[test]
fn real_account_deposit_has_exact_value_and_recoverable_owned_output() {
    let root = tempfile::tempdir().unwrap();
    let account = WalletAccount::create(&root.path().join("account")).unwrap();
    let amount = Amount::parse(Asset::Usdc, "10.25").unwrap();
    let deposit = context().prepare(account.note_keys(), amount).unwrap();
    assert_eq!(deposit.claim.pool.value, 10_250_000);
    assert_eq!(deposit.claim.pool.owner, [7; 20]);
    assert!(deposit.assumptions().unwrap().is_empty());
    let encrypted = deposit.outputs[0].encrypted().unwrap();
    let note = account
        .note_keys()
        .decrypt(&encrypted, deposit.claim.pool.domain)
        .unwrap();
    assert_eq!(note.amount(), amount);
    assert_eq!(note.recipient(), account.private_receiver().unwrap());
    let restored = SpvPoolWitness::from_bytes(&deposit.to_bytes().unwrap()).unwrap();
    assert_eq!(restored.evaluate().unwrap(), deposit.claim);
    let other = WalletAccount::create(&root.path().join("other")).unwrap();
    assert!(
        other
            .note_keys()
            .decrypt(&encrypted, deposit.claim.pool.domain)
            .is_err()
    );
}

#[test]
fn deposit_rejects_wrong_asset_zero_value_and_changed_public_amount() {
    let root = tempfile::tempdir().unwrap();
    let account = WalletAccount::create(&root.path().join("account")).unwrap();
    let context = context();
    for amount in [
        Amount::parse(Asset::Zec, "1").unwrap(),
        Amount::parse(Asset::Usdc, "0").unwrap(),
    ] {
        assert!(context.prepare(account.note_keys(), amount).is_err());
    }
    let mut deposit = context
        .prepare(
            account.note_keys(),
            Amount::parse(Asset::Usdc, "10").unwrap(),
        )
        .unwrap();
    deposit.claim.pool.value += 1;
    assert!(deposit.evaluate().is_err());
}
