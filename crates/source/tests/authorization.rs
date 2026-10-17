#![allow(clippy::unwrap_used)]

use zcash_protocol::consensus::MAIN_NETWORK;
use zutti_source::{authorization, block::ParsedBlock};

#[test]
fn coinbase_context_and_authorization_match_public_upgrade_boundaries() {
    for (height, bytes) in zebra_test::vectors::MAINNET_BLOCKS.iter() {
        if *height == 0 || *height == 434_873 {
            continue;
        }
        let block = ParsedBlock::parse(bytes, &MAIN_NETWORK).unwrap();
        let coinbase = &block.transactions()[0];
        assert_eq!(coinbase.consensus_branch_id(), block.branch());
        zutti_source::transaction::check_context(
            coinbase,
            block.branch(),
            *height,
            block.header().time,
            true,
        )
        .unwrap();
        authorization::verify(coinbase, &[]).unwrap();
    }
}

#[test]
fn sprout_groth16_authorization_matches_historical_transaction_data() {
    let mut count = 0;
    for (_, bytes) in zebra_test::vectors::MAINNET_BLOCKS.range(419_200..1_687_104) {
        let Ok(block) = ParsedBlock::parse(bytes, &MAIN_NETWORK) else {
            continue;
        };
        for tx in block.transactions() {
            if tx.sprout_bundle().is_some_and(|b| {
                b.joinsplits
                    .iter()
                    .any(|js| js.groth_proof_bytes().is_some())
            }) {
                let previous = tx.transparent_bundle().map_or_else(Vec::new, |b| {
                    vec![
                        transparent::bundle::TxOut::new(
                            zcash_protocol::value::Zatoshis::ZERO,
                            transparent::address::Script::default()
                        );
                        b.vin.len()
                    ]
                });
                authorization::verify(tx, &previous).unwrap();
                count += 1;
                if count == 3 {
                    return;
                }
            }
        }
    }
    assert!(count > 0);
}

#[test]
fn historical_sprout_proofs_verify_without_a_checkpoint_bypass() {
    let mut count = 0;
    for text in [
        include_str!("../../../tests/fixtures/block-395.txt"),
        include_str!("../../../tests/fixtures/block-396.txt"),
    ] {
        let block = ParsedBlock::parse(&hex::decode(text.trim()).unwrap(), &MAIN_NETWORK).unwrap();
        for tx in block.transactions() {
            if let Some(bundle) = tx.sprout_bundle() {
                assert!(
                    bundle
                        .joinsplits
                        .iter()
                        .all(|js| js.groth_proof_bytes().is_none())
                );
                authorization::verify(tx, &[]).unwrap();
                count += bundle.joinsplits.len();
            }
        }
    }
    assert!(count > 0);
}

#[test]
fn real_shielded_transactions_verify_with_pinned_consensus_keys() {
    let mut sapling = 0;
    let mut orchard = 0;
    for text in [
        include_str!("../../../tests/fixtures/block-1687106.txt"),
        include_str!("../../../tests/fixtures/block-1687107.txt"),
        include_str!("../../../tests/fixtures/block-1687108.txt"),
    ] {
        let block = ParsedBlock::parse(&hex::decode(text.trim()).unwrap(), &MAIN_NETWORK).unwrap();
        for tx in block.transactions() {
            if tx
                .transparent_bundle()
                .is_some_and(|b| !b.is_coinbase() && !b.vin.is_empty())
            {
                continue;
            }
            authorization::verify(tx, &[]).unwrap();
            if tx.sapling_bundle().is_some() || tx.orchard_bundle().is_some() {
                let changed = tx
                    .clone()
                    .into_data()
                    .map_bundles::<zcash_primitives::transaction::Authorized>(
                        |b| b,
                        |b| {
                            b.and_then(|b| {
                                let mut signature: [u8; 64] = b.authorization().binding_sig.into();
                                signature[0] ^= 1;
                                sapling::Bundle::from_parts(
                                    b.shielded_spends().to_vec(),
                                    b.shielded_outputs().to_vec(),
                                    *b.value_balance(),
                                    sapling::bundle::Authorized {
                                        binding_sig: signature.into(),
                                    },
                                )
                            })
                        },
                        |b| {
                            b.map(|b| {
                                b.map_authorization(
                                    &mut (),
                                    |_, _, signature| signature,
                                    |_, auth| {
                                        let mut signature: [u8; 64] =
                                            auth.binding_signature().into();
                                        signature[0] ^= 1;
                                        orchard::bundle::Authorized::from_parts(
                                            auth.proof().clone(),
                                            signature.into(),
                                        )
                                    },
                                )
                            })
                        },
                    )
                    .freeze()
                    .unwrap();
                assert!(authorization::verify(&changed, &[]).is_err());
            }
            sapling += usize::from(tx.sapling_bundle().is_some());
            orchard += usize::from(tx.orchard_bundle().is_some());
        }
    }
    assert!(sapling > 0);
    assert!(orchard > 0);
}
