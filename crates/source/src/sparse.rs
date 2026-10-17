use std::sync::Arc;
use zutti_protocol::Digest;

const DEPTH: usize = 256;
type Children = ([Arc<Node>; 2], [Digest; 2]);

#[derive(Debug, Clone, Default)]
pub(crate) struct SparseTree {
    root: Option<Arc<Node>>,
    hash: Option<Digest>,
}
#[derive(Debug)]
struct Node {
    key: Digest,
    depth: usize,
    hash: Digest,
    children: Option<Children>,
}
impl SparseTree {
    pub(crate) fn root(&self, empty: &[Digest; 257]) -> Digest {
        self.hash.unwrap_or(empty[0])
    }
    pub(crate) fn set(
        &mut self,
        key: Digest,
        value: Option<Digest>,
        empty: &[Digest; 257],
        join: &impl Fn(usize, Digest, Digest) -> Digest,
    ) {
        let leaf = value.map(|hash| {
            Arc::new(Node {
                key,
                depth: DEPTH,
                hash,
                children: None,
            })
        });
        self.root = replace(self.root.as_ref(), key, leaf, empty, join);
        self.hash = self.root.as_ref().map(|node| node.at(0, empty, join));
    }
    pub(crate) fn prove(
        &self,
        key: Digest,
        empty: &[Digest; 257],
        join: &impl Fn(usize, Digest, Digest) -> Digest,
    ) -> [Digest; 256] {
        let mut path = std::array::from_fn(|offset| empty[DEPTH - offset]);
        let mut current = self.root.as_ref();
        while let Some(node) = current {
            let common = common_bits(key, node.key).min(node.depth);
            if common < node.depth {
                path[DEPTH - 1 - common] = node.at(common + 1, empty, join);
                break;
            }
            let Some((children, hashes)) = &node.children else {
                break;
            };
            let side = bit(key, node.depth);
            path[DEPTH - 1 - node.depth] = hashes[1 - side];
            current = Some(&children[side]);
        }
        path
    }
    #[cfg(test)]
    pub(crate) fn nodes(&self) -> usize {
        fn count(node: &Node) -> usize {
            1 + node
                .children
                .as_ref()
                .map_or(0, |(children, _)| count(&children[0]) + count(&children[1]))
        }
        self.root.as_ref().map_or(0, |node| count(node))
    }
}
impl Node {
    fn at(
        &self,
        depth: usize,
        empty: &[Digest; 257],
        join: &impl Fn(usize, Digest, Digest) -> Digest,
    ) -> Digest {
        let mut current = self.hash;
        for level in (depth..self.depth).rev() {
            current = if bit(self.key, level) == 0 {
                join(level, current, empty[level + 1])
            } else {
                join(level, empty[level + 1], current)
            };
        }
        current
    }
    fn branch(
        depth: usize,
        children: [Arc<Node>; 2],
        hashes: [Digest; 2],
        join: &impl Fn(usize, Digest, Digest) -> Digest,
    ) -> Arc<Self> {
        Arc::new(Self {
            key: children[0].key,
            depth,
            hash: join(depth, hashes[0], hashes[1]),
            children: Some((children, hashes)),
        })
    }
}
fn replace(
    current: Option<&Arc<Node>>,
    key: Digest,
    leaf: Option<Arc<Node>>,
    empty: &[Digest; 257],
    join: &impl Fn(usize, Digest, Digest) -> Digest,
) -> Option<Arc<Node>> {
    let Some(node) = current else { return leaf };
    let common = common_bits(key, node.key).min(node.depth);
    if common < node.depth {
        let Some(leaf) = leaf else {
            return Some(Arc::clone(node));
        };
        let children = if bit(key, common) == 0 {
            [leaf, Arc::clone(node)]
        } else {
            [Arc::clone(node), leaf]
        };
        let hashes = std::array::from_fn(|index| children[index].at(common + 1, empty, join));
        return Some(Node::branch(common, children, hashes, join));
    }
    let Some((children, hashes)) = &node.children else {
        return leaf;
    };
    let side = bit(key, node.depth);
    let Some(child) = replace(Some(&children[side]), key, leaf, empty, join) else {
        return Some(Arc::clone(&children[1 - side]));
    };
    let mut next = children.clone();
    let mut next_hashes = *hashes;
    next_hashes[side] = child.at(node.depth + 1, empty, join);
    next[side] = child;
    Some(Node::branch(node.depth, next, next_hashes, join))
}
fn bit(key: Digest, depth: usize) -> usize {
    usize::from(key.0[depth / 8] & (1 << (7 - depth % 8)) != 0)
}
fn common_bits(left: Digest, right: Digest) -> usize {
    for (index, (left, right)) in left.0.iter().zip(right.0).enumerate() {
        let different = left ^ right;
        if different != 0 {
            return index * 8 + different.leading_zeros() as usize;
        }
    }
    DEPTH
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use zutti_protocol::commitment::hash;

    fn prefix(mut key: Digest, depth: usize) -> Digest {
        for index in depth..DEPTH {
            key.0[index / 8] &= !(1 << (7 - index % 8));
        }
        key
    }
    #[test]
    fn matches_uncompressed_roots_paths_updates_deletions_and_forks() {
        let join = |depth: usize, left: Digest, right: Digest| {
            hash(
                b"test-branch",
                &[&(depth as u16).to_le_bytes(), &left.0, &right.0],
            )
        };
        let mut empty = [Digest::default(); 257];
        empty[256] = hash(b"test-empty", &[]);
        for depth in (0..DEPTH).rev() {
            empty[depth] = join(depth, empty[depth + 1], empty[depth + 1]);
        }
        let mut tree = SparseTree::default();
        let mut nodes = BTreeMap::new();
        let mut keys = Vec::new();
        let mut entries = BTreeMap::new();
        for index in 0_u32..180 {
            let key = if index < 10 {
                let mut key = Digest::default();
                key.0[31] = index as u8;
                key
            } else {
                hash(b"test-key", &[&index.to_le_bytes()])
            };
            keys.push(key);
        }
        let mut saved = Vec::new();
        for operation in 0..540 {
            let key = keys[operation % keys.len()];
            let value = (operation < 360)
                .then(|| hash(b"test-value", &[&(operation as u32).to_le_bytes()]));
            match value {
                Some(value) => {
                    entries.insert(key, value);
                }
                None => {
                    entries.remove(&key);
                }
            }
            tree.set(key, value, &empty, &join);
            let mut current = value.unwrap_or(empty[256]);
            nodes.insert((256, key), current);
            for depth in (0..DEPTH).rev() {
                let mut sibling = prefix(key, depth + 1);
                sibling.0[depth / 8] ^= 1 << (7 - depth % 8);
                let sibling = nodes
                    .get(&(depth + 1, sibling))
                    .copied()
                    .unwrap_or(empty[depth + 1]);
                current = if bit(key, depth) == 0 {
                    join(depth, current, sibling)
                } else {
                    join(depth, sibling, current)
                };
                nodes.insert((depth, prefix(key, depth)), current);
            }
            assert_eq!(tree.root(&empty), current, "root mismatch at {operation}");
            assert_eq!(
                tree.nodes(),
                entries.len().saturating_mul(2).saturating_sub(1)
            );
            for target in [
                key,
                keys[(operation + 37) % keys.len()],
                hash(b"missing", &[&(operation as u32).to_le_bytes()]),
            ] {
                let path = tree.prove(target, &empty, &join);
                let mut digest = entries.get(&target).copied().unwrap_or(empty[256]);
                for (offset, sibling) in path.iter().enumerate() {
                    let depth = DEPTH - 1 - offset;
                    let mut sibling_key = prefix(target, depth + 1);
                    sibling_key.0[depth / 8] ^= 1 << (7 - depth % 8);
                    assert_eq!(
                        *sibling,
                        nodes
                            .get(&(depth + 1, sibling_key))
                            .copied()
                            .unwrap_or(empty[depth + 1])
                    );
                    digest = if bit(target, depth) == 0 {
                        join(depth, digest, *sibling)
                    } else {
                        join(depth, *sibling, digest)
                    };
                }
                assert_eq!(digest, current);
            }
            if operation % 90 == 0 {
                saved.push((tree.clone(), current));
            }
        }
        assert_eq!(tree.root(&empty), empty[0]);
        assert_eq!(tree.nodes(), 0);
        for (fork, root) in saved {
            assert_eq!(fork.root(&empty), root);
        }
    }
}
