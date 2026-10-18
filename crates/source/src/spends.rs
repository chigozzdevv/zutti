use crate::authenticated::{AuthenticatedMap, StateError, Transcript};
use crate::{
    block::ParsedBlock,
    trees::{CandidateTrees, SproutFrontier},
};
use group::ff::PrimeField;
use im::OrdMap;
use zcash_primitives::transaction::Transaction;
use zutti_protocol::Digest;
use zutti_protocol::{commitment::hash, encoding::Canonical};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    State(#[from] StateError),
    #[error(transparent)]
    Encoding(#[from] zutti_protocol::Error),
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
    pub(crate) state: AuthenticatedMap,
    pub(crate) counts: [u64; 4],
    pub(crate) balances: [u64; 4],
}
pub(crate) fn context() -> Digest {
    hash(b"source-shielded-state", &[])
}
fn state_key(kind: u8, pool: usize, digest: Digest) -> Digest {
    hash(b"source-shielded-key", &[&[kind, pool as u8], &digest.0])
}
impl Default for CandidateSpends {
    fn default() -> Self {
        let mut state = Self {
            state: AuthenticatedMap::new(context()),
            counts: [0; 4],
            balances: [0; 4],
        };
        if let Err(error) = state.record_roots(&CandidateTrees::default()) {
            panic!("{error}");
        }
        state
    }
}
impl CandidateSpends {
    pub const fn balances(&self) -> [u64; 4] {
        self.balances
    }
    pub fn nullifier_counts(&self) -> [usize; 4] {
        self.counts.map(|n| n as usize)
    }
    pub const fn counts(&self) -> [u64; 4] {
        self.counts
    }
    pub fn root(&self) -> Digest {
        self.state.root()
    }
    pub fn start_recording(&mut self) -> Result<()> {
        Ok(self.state.start_recording()?)
    }
    pub fn take_transcript(&mut self) -> Result<Transcript> {
        Ok(self.state.take_transcript()?)
    }
    pub fn from_witness(
        root: Digest,
        counts: [u64; 4],
        balances: [u64; 4],
        transcript: Transcript,
    ) -> Self {
        Self {
            state: AuthenticatedMap::from_witness(context(), root, transcript),
            counts,
            balances,
        }
    }
    pub fn finish_witness(&self) -> Result<()> {
        Ok(self.state.finish()?)
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
        next.record_roots(final_trees)?;
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
                let mut tree = match intermediate.get(&anchor) {
                    Some(tree) => SproutFrontier::clone(tree),
                    None => {
                        let bytes = self
                            .state
                            .get(state_key(1, 0, anchor))?
                            .ok_or(Error::Anchor)?;
                        let tree = SproutFrontier::from_bytes(&bytes)?;
                        if tree.root() != anchor {
                            return Err(Error::Anchor);
                        }
                        tree
                    }
                };
                tree.extend(js.commitments())?;
                intermediate.insert(tree.root(), tree);
            }
        }
        if let Some(bundle) = tx.sapling_bundle() {
            for spend in bundle.shielded_spends() {
                self.insert_nullifier(1, Digest(spend.nullifier().0))?;
                if self
                    .state
                    .get(state_key(1, 1, Digest(spend.anchor().to_repr())))?
                    .is_none()
                {
                    return Err(Error::Anchor);
                }
            }
        }
        for (pool, bundle) in [(2, tx.orchard_bundle()), (3, tx.ironwood_bundle())] {
            if let Some(bundle) = bundle {
                for action in bundle.actions() {
                    self.insert_nullifier(pool, Digest(action.nullifier().to_bytes()))?;
                }
                if self
                    .state
                    .get(state_key(1, pool, Digest(bundle.anchor().to_bytes())))?
                    .is_none()
                {
                    return Err(Error::Anchor);
                }
            }
        }
        Ok(())
    }
    fn insert_nullifier(&mut self, pool: usize, nf: Digest) -> Result<()> {
        if pool >= 4 {
            return Err(Error::Nullifier);
        }
        if self
            .state
            .set(state_key(0, pool, nf), Some(vec![1]))?
            .is_some()
        {
            return Err(Error::Nullifier);
        }
        self.counts[pool] = self.counts[pool].checked_add(1).ok_or(Error::Nullifier)?;
        Ok(())
    }
    fn record_roots(&mut self, trees: &CandidateTrees) -> Result<()> {
        self.state.set(
            state_key(1, 0, trees.sprout_root()),
            Some(trees.sprout_frontier().to_bytes()?),
        )?;
        let roots = trees.roots();
        for (pool, root) in [roots.sapling, roots.orchard, roots.ironwood]
            .into_iter()
            .enumerate()
        {
            self.state
                .set(state_key(1, pool + 1, root), Some(vec![1]))?;
        }
        Ok(())
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
        assert!(
            state
                .state
                .get(state_key(1, 1, roots.sapling))
                .unwrap()
                .is_some()
        );
        assert!(
            state
                .state
                .get(state_key(1, 2, roots.orchard))
                .unwrap()
                .is_some()
        );
        assert!(
            state
                .state
                .get(state_key(1, 3, roots.ironwood))
                .unwrap()
                .is_some()
        );
        assert!(
            state
                .state
                .get(state_key(1, 1, Digest([7; 32])))
                .unwrap()
                .is_none()
        );
    }
}
