use group::Curve;
use jubjub::{AffineNielsPoint, ExtendedPoint};
use std::sync::OnceLock;

const CHUNKS: usize = 172;
const CHUNKS_PER_GENERATOR: usize = 63;
static TABLE: OnceLock<Vec<AffineNielsPoint>> = OnceLock::new();

fn table() -> &'static [AffineNielsPoint] {
    TABLE.get_or_init(|| {
        let mut points = Vec::with_capacity(CHUNKS * 4);
        for (index, generator) in ::sapling::constants::PEDERSEN_HASH_GENERATORS
            .iter()
            .take(3)
            .enumerate()
        {
            let chunks = (CHUNKS - index * CHUNKS_PER_GENERATOR).min(CHUNKS_PER_GENERATOR);
            let mut base = ExtendedPoint::from(*generator);
            for _ in 0..chunks {
                let mut multiple = base;
                for _ in 0..4 {
                    points.push(multiple);
                    multiple += base;
                }
                base = base.double().double().double().double();
            }
        }
        jubjub::batch_normalize(&mut points)
            .map(|point| point.to_niels())
            .collect()
    })
}

pub(super) fn merkle_hash(depth: usize, left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    assert!(depth < 32, "invalid Sapling tree depth");
    let bit = |index: usize| match index {
        0..6 => (depth >> index) & 1,
        6..261 => usize::from((left[(index - 6) / 8] >> ((index - 6) % 8)) & 1),
        _ => usize::from((right[(index - 261) / 8] >> ((index - 261) % 8)) & 1),
    };
    let mut result = ExtendedPoint::identity();
    for (chunk, multiples) in table().chunks_exact(4).enumerate() {
        let start = chunk * 3;
        let point = &multiples[bit(start) + 2 * bit(start + 1)];
        if bit(start + 2) == 0 {
            result += point;
        } else {
            result -= point;
        }
    }
    result.to_affine().get_u().to_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_chunks_match_upstream_at_every_tree_depth() {
        for depth in 0..32 {
            for sample in 0_u8..8 {
                let left = std::array::from_fn(|index| sample.wrapping_mul(37) ^ index as u8);
                let right =
                    std::array::from_fn(|index| sample.wrapping_mul(97).wrapping_add(index as u8));
                assert_eq!(
                    merkle_hash(depth, &left, &right),
                    ::sapling::merkle_hash(depth, &left, &right)
                );
            }
            assert_eq!(
                merkle_hash(depth, &[0; 32], &[0; 32]),
                ::sapling::merkle_hash(depth, &[0; 32], &[0; 32])
            );
            assert_eq!(
                merkle_hash(depth, &[255; 32], &[255; 32]),
                ::sapling::merkle_hash(depth, &[255; 32], &[255; 32])
            );
        }
    }

    #[test]
    fn merkle_hash_uses_exactly_255_bits_of_each_child() {
        let left = [42; 32];
        let right = [91; 32];
        let mut changed_left = left;
        let mut changed_right = right;
        changed_left[31] ^= 128;
        changed_right[31] ^= 128;
        for depth in 0..32 {
            let expected = merkle_hash(depth, &left, &right);
            assert_eq!(merkle_hash(depth, &changed_left, &right), expected);
            assert_eq!(merkle_hash(depth, &left, &changed_right), expected);
            assert_ne!(merkle_hash(depth, &[0; 32], &right), expected);
        }
    }
}
