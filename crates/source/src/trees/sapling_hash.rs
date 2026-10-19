use curve::{AffineNielsPoint, ExtendedPoint};
#[cfg(not(feature = "accelerated-trees"))]
use group::Curve;
#[cfg(feature = "accelerated-trees")]
use group_legacy::Curve;
#[cfg(not(feature = "accelerated-trees"))]
use jubjub as curve;
#[cfg(feature = "accelerated-trees")]
use jubjub_legacy as curve;
use std::sync::OnceLock;

const GENERATOR_CHUNKS: [usize; 3] = [63, 63, 46];
static TABLE: OnceLock<Vec<AffineNielsPoint>> = OnceLock::new();

fn generator_point(generator: &jubjub::SubgroupPoint) -> ExtendedPoint {
    #[cfg(not(feature = "accelerated-trees"))]
    {
        ExtendedPoint::from(*generator)
    }
    #[cfg(feature = "accelerated-trees")]
    {
        let bytes = group::GroupEncoding::to_bytes(generator);
        let decoded = <curve::SubgroupPoint as group_legacy::GroupEncoding>::from_bytes(&bytes);
        match decoded.into_option() {
            Some(point) => ExtendedPoint::from(point),
            None => panic!("invalid canonical Sapling generator"),
        }
    }
}

fn table() -> &'static [AffineNielsPoint] {
    TABLE.get_or_init(|| {
        let capacity = GENERATOR_CHUNKS
            .iter()
            .map(|n| n / 2 * 32 + n % 2 * 4)
            .sum();
        let mut points = Vec::with_capacity(capacity);
        for (generator, chunks) in ::sapling::constants::PEDERSEN_HASH_GENERATORS
            .iter()
            .zip(GENERATOR_CHUNKS)
        {
            let mut base = generator_point(generator);
            for chunk in (0..chunks).step_by(2) {
                if chunk + 1 == chunks {
                    let mut multiple = base;
                    for _ in 0..4 {
                        points.push(multiple);
                        multiple += base;
                    }
                } else {
                    let high_base = base.double().double().double().double();
                    let mut high = high_base;
                    for _ in 0..4 {
                        let mut low = base;
                        for _ in 0..4 {
                            points.push(high + low);
                            points.push(high - low);
                            low += base;
                        }
                        high += high_base;
                    }
                    base = high_base.double().double().double().double();
                }
            }
        }
        curve::batch_normalize(&mut points)
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
    let points = table();
    let mut result = ExtendedPoint::identity();
    let mut start = 0;
    let mut offset = 0;
    for chunks in GENERATOR_CHUNKS {
        for chunk in (0..chunks).step_by(2) {
            let low = bit(start) + 2 * bit(start + 1);
            let (index, sign) = if chunk + 1 == chunks {
                offset += 4;
                (offset - 4 + low, bit(start + 2))
            } else {
                let high = bit(start + 3) + 2 * bit(start + 4);
                let sign = bit(start + 5);
                let index = offset + high * 8 + low * 2 + (bit(start + 2) ^ sign);
                offset += 32;
                (index, sign)
            };
            if sign == 0 {
                result += &points[index];
            } else {
                result -= &points[index];
            }
            start += if chunk + 1 == chunks { 3 } else { 6 };
        }
    }
    result.to_affine().get_u().to_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paired_windows_match_all_signs_and_generator_boundaries() {
        for start in [6, 186, 189, 375, 378, 510] {
            for pattern in 0_u8..64 {
                let mut left = [0; 32];
                let mut right = [0; 32];
                for bit in 0..6 {
                    let index = start + bit - 6;
                    let (child, position) = if index < 255 {
                        (&mut left, index)
                    } else {
                        (&mut right, index - 255)
                    };
                    child[position / 8] |= ((pattern >> bit) & 1) << (position % 8);
                }
                assert_eq!(
                    merkle_hash(17, &left, &right),
                    ::sapling::merkle_hash(17, &left, &right)
                );
            }
        }
    }

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
