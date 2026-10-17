#![allow(clippy::unwrap_used)]
#[path = "../../../tests/support/mod.rs"]
mod support;
use support::{digest, input, secret, terms};
use zutti_protocol::{encoding::Canonical, order::OrderTerms};
use zutti_source::{
    CandidateIndex, IndexContext, IndexProof, IndexSnapshot,
    consensus::verify_history,
    query::{QueryClaim, QueryWitness},
};

fn tree(end: u32) -> CandidateIndex {
    let domain = support::domain();
    let mut tree = CandidateIndex::new(
        IndexContext::new(domain.source_genesis(), domain.source_policy(), 10).unwrap(),
    );
    for height in 11..=end {
        tree.append_block(height, &[digest(height as u8)]).unwrap();
    }
    tree
}
fn witness(tree: &CandidateIndex, tx: u8) -> QueryWitness {
    QueryWitness::new(
        terms(),
        secret(31),
        secret(32),
        digest(tx),
        tree.prove(digest(tx)),
        tree.snapshot(),
    )
}

#[test]
fn terminal_queries_keep_private_openings_out_of_the_public_claim() {
    let tree = tree(30);
    let paid = witness(&tree, 20).evaluate().unwrap();
    let absent = witness(&tree, 90).evaluate().unwrap();
    assert_eq!(paid.output(), support::terms().payout().commitment());
    assert_eq!(absent.output(), support::terms().refund().commitment());
    assert_eq!(paid.order(), absent.order());
    assert_ne!(paid.reservation(), absent.reservation());
    for (claim, tx) in [(paid, 20), (absent, 90)] {
        let bytes = claim.to_bytes().unwrap();
        assert_eq!(QueryClaim::from_bytes(&bytes).unwrap(), claim);
        for private in [digest(tx).0, [31; 32], [32; 32], [8; 32]] {
            assert!(!bytes.windows(32).any(|part| part == private));
        }
        assert!(verify_history(claim.snapshot(), &bytes).is_err());
    }
}

#[test]
fn expiry_is_inclusive_and_paid_claims_survive_delayed_proving() {
    let at_expiry = tree(20);
    assert_eq!(
        witness(&at_expiry, 20).evaluate().unwrap().output(),
        support::terms().payout().commitment()
    );
    assert!(witness(&at_expiry, 90).evaluate().is_err());
    let after_expiry = tree(21);
    assert_eq!(
        witness(&after_expiry, 90).evaluate().unwrap().output(),
        support::terms().refund().commitment()
    );
    assert_eq!(
        witness(&tree(30), 20).evaluate().unwrap().output(),
        support::terms().payout().commitment()
    );
    for tx in [11, 21, 30] {
        assert!(witness(&tree(30), tx).evaluate().is_err());
    }
}

#[test]
fn modified_witness_or_history_cannot_reuse_the_same_claim() {
    let tree = tree(21);
    let original = witness(&tree, 20);
    let claim = original.evaluate().unwrap();
    let make = |snapshot, path, order_secret, reservation_secret, txid| {
        QueryWitness::new(
            terms(),
            secret(order_secret),
            secret(reservation_secret),
            digest(txid),
            path,
            snapshot,
        )
    };
    for (a, b) in [(33, 32), (31, 33)] {
        assert_ne!(
            make(tree.snapshot(), tree.prove(digest(20)), a, b, 20)
                .evaluate()
                .unwrap(),
            claim
        );
    }
    assert!(
        make(tree.snapshot(), tree.prove(digest(20)), 31, 32, 90)
            .evaluate()
            .is_err()
    );
    let mut snapshot = tree.snapshot();
    snapshot.root = digest(99);
    assert!(
        make(snapshot, tree.prove(digest(20)), 31, 32, 20)
            .evaluate()
            .is_err()
    );
    let mut path = tree.prove(digest(20)).to_bytes().unwrap();
    let last = path.len() - 1;
    path[last] ^= 1;
    assert!(
        make(
            tree.snapshot(),
            IndexProof::from_bytes(&path).unwrap(),
            31,
            32,
            20
        )
        .evaluate()
        .is_err()
    );
    for change_genesis in [true, false] {
        let domain = support::domain();
        let context = IndexContext::new(
            if change_genesis {
                digest(90)
            } else {
                domain.source_genesis()
            },
            if change_genesis {
                domain.source_policy()
            } else {
                digest(90)
            },
            10,
        )
        .unwrap();
        assert!(
            make(
                IndexSnapshot {
                    context,
                    ..tree.snapshot()
                },
                tree.prove(digest(20)),
                31,
                32,
                20
            )
            .evaluate()
            .is_err()
        );
    }
    let mut changed = input();
    changed.receiver = [90; 43];
    assert_ne!(
        QueryWitness::new(
            OrderTerms::new(changed).unwrap(),
            secret(31),
            secret(32),
            digest(20),
            tree.prove(digest(20)),
            tree.snapshot()
        )
        .evaluate()
        .unwrap(),
        claim
    );
}

#[test]
fn private_witness_encoding_rejects_truncation_and_trailing_data() {
    let original = witness(&tree(21), 20);
    let mut bytes = original.to_bytes().unwrap();
    assert_eq!(
        QueryWitness::from_bytes(&bytes)
            .unwrap()
            .evaluate()
            .unwrap(),
        original.evaluate().unwrap()
    );
    assert_eq!(format!("{original:?}"), "QueryWitness([REDACTED])");
    for end in [0, 8, 128, bytes.len() - 1] {
        assert!(QueryWitness::from_bytes(&bytes[..end]).is_err());
    }
    bytes.push(0);
    assert!(QueryWitness::from_bytes(&bytes).is_err());
}
