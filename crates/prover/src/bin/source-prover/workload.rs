use std::collections::BTreeMap;
use zcash_protocol::consensus::{MAIN_NETWORK, NetworkType, TEST_NETWORK};
use zutti_source::{block::ParsedBlock, proof::SourceInput, range::SourceRangeWitness};

pub(super) fn describe(
    witness: &SourceRangeWitness,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let SourceRangeWitness::Transition(witness) = witness else {
        return Err("a source transition is required".into());
    };
    let SourceInput::Advance { transition, .. } = &witness.input else {
        return Err("a source transition is required".into());
    };
    let mut versions = BTreeMap::<u32, usize>::new();
    let mut transactions = 0;
    let mut bytes = 0;
    let mut sprout_joinsplits = 0;
    let mut sapling_spends = 0;
    let mut sapling_outputs = 0;
    let mut orchard_actions = 0;
    let mut ironwood_actions = 0;
    for raw in transition.blocks() {
        let block = match witness.policy.network {
            NetworkType::Main => ParsedBlock::parse(raw, &MAIN_NETWORK)?,
            NetworkType::Test => ParsedBlock::parse(raw, &TEST_NETWORK)?,
            _ => return Err("unsupported source network".into()),
        };
        bytes += raw.len();
        transactions += block.txids().len();
        for version in block.versions() {
            *versions.entry(*version).or_default() += 1;
        }
        for tx in block.transactions() {
            sprout_joinsplits += tx.sprout_bundle().map_or(0, |b| b.joinsplits.len());
            if let Some(bundle) = tx.sapling_bundle() {
                sapling_spends += bundle.shielded_spends().len();
                sapling_outputs += bundle.shielded_outputs().len();
            }
            orchard_actions += tx.orchard_bundle().map_or(0, |b| b.actions().len());
            ironwood_actions += tx.ironwood_bundle().map_or(0, |b| b.actions().len());
        }
    }
    Ok(
        serde_json::json!({"blocks":transition.block_count(),"block_bytes":bytes,
        "transactions":transactions,"non_coinbase_transactions":transactions-transition.block_count(),
        "transaction_versions":versions,"sprout_joinsplits":sprout_joinsplits,
        "sapling_spends":sapling_spends,"sapling_outputs":sapling_outputs,
        "orchard_actions":orchard_actions,"ironwood_actions":ironwood_actions}),
    )
}
