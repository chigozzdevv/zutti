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
use rand_core::OsRng;
use zcash_primitives::transaction::{
    builder::{BuildConfig, Builder, BundlePadding},
    fees::zip317,
};
use zcash_protocol::{
    consensus::BlockHeight, local_consensus::LocalNetwork, memo::MemoBytes, value::Zatoshis,
};
use zutti_protocol::{Secret32, order::OrderTerms, policy::PaymentWindow};
fn network() -> LocalNetwork {
    LocalNetwork {
        overwinter: Some(1.into()),
        sapling: Some(2.into()),
        blossom: Some(3.into()),
        heartwood: Some(4.into()),
        canopy: Some(5.into()),
        nu5: Some(6.into()),
        nu6: Some(7.into()),
        nu6_1: Some(8.into()),
        nu6_2: Some(9.into()),
        nu6_3: Some(10.into()),
    }
}
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
    let key = SpendingKey::from_bytes([7; 32]).unwrap();
    let fvk = FullViewingKey::from(&key);
    let payer_receiver = fvk.address_at(0_u32, Scope::External);
    let recipient_key = SpendingKey::from_bytes([8; 32]).unwrap();
    let recipient = FullViewingKey::from(&recipient_key).address_at(0_u32, Scope::External);
    let mut input = support::input();
    input.receiver = recipient.to_raw_address_bytes();
    input.window = PaymentWindow::new(10, 11, support::digest(13), 120).unwrap();
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
        network(),
        BlockHeight::from_u32(100),
        BuildConfig::Standard {
            sapling_anchor: None,
            orchard_anchor: None,
            ironwood_anchor: Some(anchor),
            orchard_padding: BundlePadding::DEFAULT,
            ironwood_padding: BundlePadding::DEFAULT,
        },
    )
    .with_expiry_height(120.into());
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
        .build_for_pczt(OsRng, &zip317::FeeRule::standard())
        .unwrap();
    let spend = result.ironwood_meta.spend_action_index(0).unwrap();
    let pczt = IoFinalizer::new(Creator::build_from_parts(result.pczt_parts).unwrap())
        .finalize_io()
        .unwrap();
    Fixture {
        terms,
        randomness,
        pczt,
        spend,
        key,
    }
}
