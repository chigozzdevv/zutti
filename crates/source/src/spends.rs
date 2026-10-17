use crate::{
    block::ParsedBlock,
    trees::{CandidateTrees, SproutFrontier},
};
use group::ff::PrimeField;
use im::{OrdMap, OrdSet};
use std::sync::Arc;
use zcash_primitives::transaction::Transaction;
use zutti_protocol::Digest;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("shielded nullifier has already been used")]
    Nullifier,
    #[error("shielded anchor is not an earlier final tree or permitted Sprout intermediate tree")]
    Anchor,
    #[error("shielded value pool would become negative or exceed the monetary limit")]
    Value,
    #[error(transparent)]
    Tree(#[from] crate::trees::Error),
}
type Result<T> = core::result::Result<T, Error>;

#[derive(Debug, Clone)]
pub struct CandidateSpends {
    nullifiers: [OrdSet<Digest>; 4],
    sprout_anchors: OrdMap<Digest, Arc<SproutFrontier>>,
    anchors: [OrdSet<Digest>; 3],
    balances: [u64; 4],
}
impl Default for CandidateSpends {
    fn default() -> Self {
        let mut state = Self {
            nullifiers: std::array::from_fn(|_| OrdSet::new()),
            sprout_anchors: OrdMap::new(),
            anchors: std::array::from_fn(|_| OrdSet::new()),
            balances: [0; 4],
        };
        state.record_roots(&CandidateTrees::default());
        state
    }
}
impl CandidateSpends {
    pub const fn balances(&self) -> [u64; 4] {
        self.balances
    }
    pub fn nullifier_counts(&self) -> [usize; 4] {
        self.nullifiers.each_ref().map(OrdSet::len)
    }
    pub fn transition(&self, block: &ParsedBlock, final_trees: &CandidateTrees) -> Result<Self> {
        let mut next = self.clone();
        let mut deltas = [0_i128; 4];
        for tx in block.transactions() {
            next.check_spends(tx)?;
            if let Some(bundle) = tx.sprout_bundle() {
                for js in &bundle.joinsplits {
                    deltas[0] += i128::from(i64::from(js.net_value()));
                }
            }
            deltas[1] += tx
                .sapling_bundle()
                .map_or(0, |b| i128::from(i64::from(*b.value_balance())));
            deltas[2] += tx
                .orchard_bundle()
                .map_or(0, |b| i128::from(i64::from(*b.value_balance())));
            deltas[3] += tx
                .ironwood_bundle()
                .map_or(0, |b| i128::from(i64::from(*b.value_balance())));
        }
        for (balance, delta) in next.balances.iter_mut().zip(deltas) {
            *balance = u64::try_from(i128::from(*balance) - delta)
                .ok()
                .filter(|value| *value <= crate::transaction::MAX_MONEY)
                .ok_or(Error::Value)?;
        }
        next.record_roots(final_trees);
        Ok(next)
    }
    fn check_spends(&mut self, tx: &Transaction) -> Result<()> {
        if let Some(bundle) = tx.sprout_bundle() {
            let mut intermediate = OrdMap::new();
            for js in &bundle.joinsplits {
                for nf in js.nullifiers() {
                    self.insert_nullifier(0, Digest(*nf))?;
                }
                let anchor = Digest(*js.anchor());
                let previous = intermediate
                    .get(&anchor)
                    .or_else(|| self.sprout_anchors.get(&anchor))
                    .ok_or(Error::Anchor)?;
                let mut tree: SproutFrontier = (**previous).clone();
                tree.extend(js.commitments())?;
                intermediate.insert(tree.root(), Arc::new(tree));
            }
        }
        if let Some(bundle) = tx.sapling_bundle() {
            for spend in bundle.shielded_spends() {
                self.insert_nullifier(1, Digest(spend.nullifier().0))?;
                if !self.anchors[0].contains(&Digest(spend.anchor().to_repr())) {
                    return Err(Error::Anchor);
                }
            }
        }
        for (pool, bundle) in [(2, tx.orchard_bundle()), (3, tx.ironwood_bundle())] {
            if let Some(bundle) = bundle {
                for action in bundle.actions() {
                    self.insert_nullifier(pool, Digest(action.nullifier().to_bytes()))?;
                }
                if !self.anchors[pool - 1].contains(&Digest(bundle.anchor().to_bytes())) {
                    return Err(Error::Anchor);
                }
            }
        }
        Ok(())
    }
    fn insert_nullifier(&mut self, pool: usize, nf: Digest) -> Result<()> {
        if self.nullifiers[pool].insert(nf).is_some() {
            return Err(Error::Nullifier);
        }
        Ok(())
    }
    fn record_roots(&mut self, trees: &CandidateTrees) {
        self.sprout_anchors
            .entry(trees.sprout_root())
            .or_insert_with(|| Arc::new(trees.sprout_frontier()));
        let roots = trees.roots();
        for (anchors, root) in
            self.anchors
                .iter_mut()
                .zip([roots.sapling, roots.orchard, roots.ironwood])
        {
            anchors.insert(root);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn nullifiers_are_unique_within_each_independent_pool() {
        let mut state = CandidateSpends::default();
        for pool in 0..4 {
            state.insert_nullifier(pool, Digest([1; 32])).unwrap();
        }
        assert_eq!(state.nullifier_counts(), [1; 4]);
        for pool in 0..4 {
            assert!(matches!(
                state.insert_nullifier(pool, Digest([1; 32])),
                Err(Error::Nullifier)
            ));
        }
    }
    #[test]
    fn only_empty_genesis_trees_are_available_initially() {
        let state = CandidateSpends::default();
        assert_eq!(state.balances(), [0; 4]);
        let roots = CandidateTrees::default().roots();
        assert!(state.anchors[0].contains(&roots.sapling));
        assert!(state.anchors[1].contains(&roots.orchard));
        assert!(state.anchors[2].contains(&roots.ironwood));
        assert!(!state.anchors[0].contains(&Digest([7; 32])));
    }
}
