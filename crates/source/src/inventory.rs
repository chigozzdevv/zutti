use crate::{Error, Result, block::ParsedBlock};
use std::collections::BTreeMap;
use zcash_protocol::consensus::{BranchId, Parameters};
use zutti_protocol::Digest;

pub const COUNT_FIELDS: &[&str] = &[
    "transactions",
    "non_coinbase_transactions",
    "transparent_inputs",
    "transparent_outputs",
    "sprout_phgr_proofs",
    "sprout_groth16_proofs",
    "sprout_signatures",
    "sapling_bundles",
    "sapling_spends",
    "sapling_outputs",
    "orchard_bundles",
    "orchard_actions",
    "ironwood_bundles",
    "ironwood_actions",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockWorkload {
    pub height: u32,
    pub hash: Digest,
    pub parent: Digest,
    pub branch: BranchId,
    pub bytes: usize,
    pub counts: BTreeMap<&'static str, u64>,
    pub versions: BTreeMap<u32, u64>,
}

impl BlockWorkload {
    fn from_block(block: &ParsedBlock, bytes: usize) -> Self {
        let mut counts: BTreeMap<_, _> = COUNT_FIELDS.iter().map(|field| (*field, 0)).collect();
        let mut versions = BTreeMap::new();
        for transaction in block.transactions() {
            counts.entry("transactions").and_modify(|n| *n += 1);
            *versions
                .entry(transaction.version().header() & 0x7fff_ffff)
                .or_insert(0) += 1;
            let coinbase = transaction
                .transparent_bundle()
                .is_some_and(|bundle| bundle.is_coinbase());
            if !coinbase {
                counts
                    .entry("non_coinbase_transactions")
                    .and_modify(|n| *n += 1);
            }
            if let Some(bundle) = transaction.transparent_bundle() {
                if !coinbase {
                    counts
                        .entry("transparent_inputs")
                        .and_modify(|n| *n += bundle.vin.len() as u64);
                }
                counts
                    .entry("transparent_outputs")
                    .and_modify(|n| *n += bundle.vout.len() as u64);
            }
            if let Some(bundle) = transaction.sprout_bundle() {
                counts.entry("sprout_signatures").and_modify(|n| *n += 1);
                for joinsplit in &bundle.joinsplits {
                    let kind = if joinsplit.groth_proof_bytes().is_some() {
                        "sprout_groth16_proofs"
                    } else {
                        "sprout_phgr_proofs"
                    };
                    counts.entry(kind).and_modify(|n| *n += 1);
                }
            }
            if let Some(bundle) = transaction.sapling_bundle() {
                counts.entry("sapling_bundles").and_modify(|n| *n += 1);
                counts
                    .entry("sapling_spends")
                    .and_modify(|n| *n += bundle.shielded_spends().len() as u64);
                counts
                    .entry("sapling_outputs")
                    .and_modify(|n| *n += bundle.shielded_outputs().len() as u64);
            }
            for (kind, actions, bundle) in [
                (
                    "orchard_bundles",
                    "orchard_actions",
                    transaction.orchard_bundle(),
                ),
                (
                    "ironwood_bundles",
                    "ironwood_actions",
                    transaction.ironwood_bundle(),
                ),
            ] {
                if let Some(bundle) = bundle {
                    counts.entry(kind).and_modify(|n| *n += 1);
                    counts
                        .entry(actions)
                        .and_modify(|n| *n += bundle.actions().len() as u64);
                }
            }
        }
        Self {
            height: block.claimed_height(),
            hash: block.hash(),
            parent: block.parent(),
            branch: block.branch(),
            bytes,
            counts,
            versions,
        }
    }
}

#[derive(Debug)]
pub struct InventoryChain {
    next_height: u32,
    parent: Digest,
}

impl InventoryChain {
    pub const fn new(next_height: u32, parent: Digest) -> Self {
        Self {
            next_height,
            parent,
        }
    }

    pub fn append<P: Parameters>(&mut self, bytes: &[u8], parameters: &P) -> Result<BlockWorkload> {
        let block = ParsedBlock::parse(bytes, parameters)?;
        if block.claimed_height() != self.next_height {
            return Err(Error::Height);
        }
        if block.parent() != self.parent {
            return Err(Error::Parent);
        }
        let next_height = self.next_height.checked_add(1).ok_or(Error::Height)?;
        let workload = BlockWorkload::from_block(&block, bytes.len());
        self.parent = block.hash();
        self.next_height = next_height;
        Ok(workload)
    }
}
