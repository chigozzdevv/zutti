use super::support;
use incrementalmerkletree::{Hashable, Level};
use orchard::{
    Note,
    keys::{FullViewingKey, Scope, SpendingKey},
    note::{NoteVersion, RandomSeed, Rho},
    tree::{MerkleHashOrchard, MerklePath},
    value::NoteValue,
};
use pczt::{
    Pczt,
    roles::{creator::Creator, io_finalizer::IoFinalizer},
};
use rand::{SeedableRng, rngs::StdRng};
use zcash_primitives::transaction::{
    builder::{BuildConfig, Builder, BundlePadding},
    fees::zip317,
};
use zcash_protocol::{
    consensus::{BlockHeight, TEST_NETWORK},
    memo::MemoBytes,
    value::Zatoshis,
};
use zutti_protocol::{
    Secret32, domain::Domain, network::ZCASH_TESTNET_GENESIS, order::OrderTerms,
    policy::PaymentWindow,
};
#[allow(dead_code)]
pub struct Fixture {
    pub terms: OrderTerms,
    pub randomness: Secret32,
    pub pczt: Pczt,
    pub spend: usize,
    pub key: SpendingKey,
}
pub fn fixture(wrong_memo: bool) -> Fixture {
    fixture_with_cap(wrong_memo, "0.001")
}
pub fn fixture_with_cap(wrong_memo: bool, fee_cap: &str) -> Fixture {
    fixture_at(wrong_memo, fee_cap, 4_465_101)
}
pub fn fixture_at(wrong_memo: bool, fee_cap: &str, height: u32) -> Fixture {
    let key = SpendingKey::from_bytes([7; 32]).unwrap();
    let fvk = FullViewingKey::from(&key);
    let payer_receiver = fvk.address_at(0_u32, Scope::External);
    let recipient_key = SpendingKey::from_bytes([8; 32]).unwrap();
    let recipient = FullViewingKey::from(&recipient_key).address_at(0_u32, Scope::External);
    let mut input = support::input();
    input.domain = Domain::new(
        ZCASH_TESTNET_GENESIS,
        11155111,
        [2; 20],
        [3; 20],
        support::digest(4),
        support::digest(5),
        support::digest(6),
    )
    .unwrap();
    input.receiver = recipient.to_raw_address_bytes();
    input.window = PaymentWindow::new(0, height - 1, support::digest(13), height + 20).unwrap();
    input.fee_caps[0] =
        zutti_protocol::amount::Amount::parse(zutti_protocol::amount::Asset::Zec, fee_cap).unwrap();
    let terms = OrderTerms::new(input).unwrap();
    let randomness = support::secret(21);
    let mut memo = terms.memo(terms.commitment(&randomness).unwrap()).unwrap();
    assert!(matches!(
        zcash_protocol::memo::Memo::try_from(MemoBytes::from_bytes(&memo).unwrap()),
        Ok(zcash_protocol::memo::Memo::Arbitrary(_))
    ));
    if wrong_memo {
        memo[42] ^= 1;
    }
    let rho = Rho::from_bytes(&[1; 32]).into_option().unwrap();
    let rseed = (0..=255_u8)
        .find_map(|n| RandomSeed::from_bytes([n; 32], &rho).into_option())
        .unwrap();
    let note = Note::from_parts(
        payer_receiver,
        NoteValue::from_raw(500_010_000),
        rho,
        rseed,
        NoteVersion::V3,
    )
    .into_option()
    .unwrap();
    let cmx = note.commitment().into();
    let path = MerklePath::from_parts(
        0,
        std::array::from_fn(|i| MerkleHashOrchard::empty_root(Level::from(i as u8))),
    );
    let anchor = path.root(cmx);
    let mut builder = Builder::new(
        TEST_NETWORK,
        BlockHeight::from_u32(height),
        BuildConfig::Standard {
            sapling_anchor: None,
            orchard_anchor: None,
            ironwood_anchor: Some(anchor),
            orchard_padding: BundlePadding::DEFAULT,
            ironwood_padding: BundlePadding::DEFAULT,
        },
    )
    .with_expiry_height((height + 20).into());
    builder
        .add_ironwood_spend::<zip317::FeeRule>(fvk.clone(), note, path)
        .unwrap();
    builder
        .add_ironwood_output::<zip317::FeeRule>(
            Some(fvk.to_ovk(Scope::External)),
            recipient,
            Zatoshis::const_from_u64(500_000_000),
            MemoBytes::from_bytes(&memo).unwrap(),
        )
        .unwrap();
    let result = builder
        .build_for_pczt(StdRng::from_seed([42; 32]), &zip317::FeeRule::standard())
        .unwrap();
    let spend = result.ironwood_meta.spend_action_index(0).unwrap();
    let pczt = IoFinalizer::new(Creator::build_from_parts(result.pczt_parts).unwrap())
        .finalize_io(StdRng::from_seed([43; 32]))
        .unwrap();
    Fixture {
        terms,
        randomness,
        pczt,
        spend,
        key,
    }
}
