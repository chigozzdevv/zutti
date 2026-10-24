#![allow(clippy::unwrap_used)]
use zcash_protocol::consensus::NetworkType;
use zutti_protocol::{Digest, Secret32, encoding::Canonical};
use zutti_source::spv::{
    CoverageClaim, CoveragePrevious, CoverageWitness, Evaluation, HEADER_BYTES, HeaderClaim,
    Policy, WindowClaim, WindowTerms, Witness, parse_block,
};
fn digest(byte: u8) -> Digest {
    Digest([byte; 32])
}
fn secret() -> Secret32 {
    Secret32::from_bytes([7; 32]).unwrap()
}
fn block(height: u32) -> Vec<u8> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = root.join("../..");
    let file = if height == 0 {
        root.join("tests/fixtures/block-test-0.txt")
    } else {
        root.join(format!("tests/fixtures/spv/testnet-{height}.hex"))
    };
    hex::decode(std::fs::read_to_string(file).unwrap().trim()).unwrap()
}
fn prefix() -> HeaderClaim {
    let genesis = block(0);
    HeaderClaim::genesis(
        Policy::new(digest(1), NetworkType::Test, 1, 3, 4).unwrap(),
        &genesis[..HEADER_BYTES],
    )
    .unwrap()
    .advance(&[block(1)[..HEADER_BYTES].to_vec()])
    .unwrap()
}
fn terms() -> WindowTerms {
    let head = prefix();
    WindowTerms::new(
        digest(2),
        digest(3),
        1,
        head.tip().unwrap(),
        3,
        digest(4),
        digest(5),
    )
    .unwrap()
}
fn txid(height: u32) -> Digest {
    parse_block(&block(height), NetworkType::Test)
        .unwrap()
        .txids()[0]
}
fn start(target: Digest) -> CoverageWitness {
    CoverageWitness {
        terms: terms(),
        txid: target,
        randomness: secret(),
        previous: CoveragePrevious::Prefix(prefix()),
        blocks: vec![block(2)],
        confirmations: vec![],
        finish: false,
    }
}
fn partial(witness: &CoverageWitness) -> CoverageClaim {
    let Evaluation::Coverage(claim) = witness.evaluate().unwrap() else {
        panic!("coverage")
    };
    claim
}
fn finish(previous: CoverageClaim, target: Digest) -> CoverageWitness {
    CoverageWitness {
        terms: terms(),
        txid: target,
        randomness: secret(),
        previous: CoveragePrevious::Coverage(previous),
        blocks: vec![block(3)],
        confirmations: vec![block(4)[..HEADER_BYTES].to_vec()],
        finish: true,
    }
}
fn window(witness: CoverageWitness) -> WindowClaim {
    let Evaluation::Window(claim) = witness.evaluate().unwrap() else {
        panic!("window")
    };
    claim
}
#[test]
fn consecutive_chunks_produce_complete_paid_or_absent_window_observations() {
    for (target, expected) in [
        (txid(2), digest(4)),
        (txid(3), digest(4)),
        (txid(4), digest(5)),
    ] {
        let first = start(target);
        let coverage = partial(&first);
        assert_eq!(coverage.head.height().unwrap(), 2);
        assert_eq!(coverage.payment, if target == txid(2) { 2 } else { 0 });
        let final_witness = finish(coverage, target);
        let observed = window(final_witness);
        assert_eq!(observed.output, expected);
        assert_eq!(observed.prefix, prefix().id().unwrap());
        assert_eq!(observed.height, 4);
        assert_eq!(
            observed.reservation,
            terms()
                .reservation(prefix().policy(), target, &secret())
                .unwrap()
        );
    }
}
#[test]
fn a_paid_prefix_cannot_be_forgotten_by_a_later_absent_chunk() {
    let first = partial(&start(txid(2)));
    assert_eq!(first.payment, 2);
    assert_eq!(window(finish(first, txid(2))).output, digest(4));
}
#[test]
fn changed_target_salt_terms_anchor_or_prior_identity_cannot_be_spliced() {
    let prior = partial(&start(txid(4)));
    for field in 0..5 {
        let mut witness = finish(prior.clone(), txid(4));
        match field {
            0 => witness.txid = txid(2),
            1 => witness.randomness = Secret32::from_bytes([8; 32]).unwrap(),
            2 => {
                witness.terms = WindowTerms::new(
                    digest(2),
                    digest(6),
                    1,
                    prefix().tip().unwrap(),
                    3,
                    digest(4),
                    digest(5),
                )
                .unwrap()
            }
            3 => witness.previous = CoveragePrevious::Prefix(prefix()),
            _ => witness.blocks = vec![block(2)],
        }
        assert!(witness.evaluate().is_err(), "field {field}");
    }
    let mut wrong = start(txid(4));
    wrong.terms =
        WindowTerms::new(digest(2), digest(3), 1, digest(9), 3, digest(4), digest(5)).unwrap();
    assert!(wrong.evaluate().is_err());
}
#[test]
fn missing_reordered_incomplete_and_unconfirmed_windows_fail() {
    let prior = partial(&start(txid(4)));
    for field in 0..4 {
        let mut witness = finish(prior.clone(), txid(4));
        match field {
            0 => witness.blocks.clear(),
            1 => witness.confirmations.clear(),
            2 => witness.blocks = vec![block(4)],
            _ => witness.blocks = vec![block(3), block(3)],
        }
        assert!(witness.evaluate().is_err(), "field {field}");
    }
}
#[test]
fn coverage_encoding_roundtrips_and_binds_its_exact_dependency_journal() {
    assert_eq!(
        format!("{:?}", partial(&start(txid(2)))),
        format!("{:?}", partial(&start(txid(4)))),
    );
    let initial = start(txid(4));
    let prior = partial(&initial);
    let bytes = prior.to_bytes().unwrap();
    assert!(WindowClaim::from_bytes(&bytes).is_err());
    let next = Witness::Coverage(Box::new(finish(
        CoverageClaim::from_bytes(&bytes).unwrap(),
        txid(4),
    )));
    assert_eq!(next.required_journal().unwrap().unwrap(), bytes);
    assert!(next.is_private());
    let serialized = next.to_bytes().unwrap();
    assert_eq!(
        Witness::from_bytes(&serialized)
            .unwrap()
            .evaluate()
            .unwrap()
            .journal()
            .unwrap(),
        next.evaluate().unwrap().journal().unwrap()
    );
    let mut trailing = serialized;
    trailing.push(0);
    assert!(Witness::from_bytes(&trailing).is_err());
}
