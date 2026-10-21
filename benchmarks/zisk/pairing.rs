struct PairingResult(bool);

impl PairingResult {
    fn is_zero(&self) -> bool {
        self.0
    }
}

fn pairing<const N: usize>(points: [G1Affine; N], twists: [G2Affine; N]) -> PairingResult {
    let g1 = points.map(|point| {
        if point.is_zero() {
            return [0; 8];
        }
        let mut limbs = [0; 8];
        limbs[..4].copy_from_slice(&point.x.into_bigint().0);
        limbs[4..].copy_from_slice(&point.y.into_bigint().0);
        limbs
    });
    let g2 = twists.map(|point| {
        if point.is_zero() {
            return [0; 16];
        }
        let mut limbs = [0; 16];
        limbs[..4].copy_from_slice(&point.x.c0.into_bigint().0);
        limbs[4..8].copy_from_slice(&point.x.c1.into_bigint().0);
        limbs[8..12].copy_from_slice(&point.y.c0.into_bigint().0);
        limbs[12..].copy_from_slice(&point.y.c1.into_bigint().0);
        limbs
    });
    let value = ziskos::zisklib::pairing_batch_bn254(&g1, &g2);
    PairingResult(value[0] == 1 && value[1..].iter().all(|word| *word == 0))
}
