#![allow(clippy::unwrap_used)]
use zutti_protocol::{Digest, encoding::Canonical, policy::PaymentWindow};
use zutti_source::{
    CandidateIndex, Error, IndexContext, IndexProof, Observation,
    consensus::verify_history,
    history::{CandidateOutcome, classify_candidate},
};

fn key(n: u8) -> Digest {
    Digest([n; 32])
}
fn index() -> CandidateIndex {
    CandidateIndex::new(IndexContext::new(key(1), key(2), 10).unwrap())
}
fn window() -> PaymentWindow {
    PaymentWindow::new(10, 11, key(3), 20).unwrap()
}

#[test]
fn exact_expiry_boundary_and_delayed_paid_claim() {
    let mut tree = index();
    for height in 11..=20 {
        tree.append_block(height, &[key(height as u8)]).unwrap();
    }
    assert_eq!(
        classify_candidate(window(), tree.snapshot(), &tree.prove(key(20)), key(20)).unwrap(),
        CandidateOutcome::Paid { height: 20 }
    );
    assert_eq!(
        classify_candidate(window(), tree.snapshot(), &tree.prove(key(80)), key(80)).unwrap(),
        CandidateOutcome::Waiting
    );
    for height in 21..=30 {
        tree.append_block(height, &[key(height as u8)]).unwrap();
    }
    assert_eq!(
        classify_candidate(window(), tree.snapshot(), &tree.prove(key(20)), key(20)).unwrap(),
        CandidateOutcome::Paid { height: 20 }
    );
    assert_eq!(
        classify_candidate(window(), tree.snapshot(), &tree.prove(key(80)), key(80)).unwrap(),
        CandidateOutcome::ExpiredAbsent
    );
    assert!(classify_candidate(window(), tree.snapshot(), &tree.prove(key(21)), key(21)).is_err());
}
#[test]
fn changed_root_key_context_height_or_path_is_rejected() {
    let mut tree = index();
    tree.append_block(11, &[key(20), key(90)]).unwrap();
    let proof = tree.prove(key(20));
    assert_eq!(
        proof.verify(tree.snapshot(), key(20)).unwrap(),
        Observation::Present(11)
    );
    assert!(proof.verify(tree.snapshot(), key(21)).is_err());
    let mut changed = tree.snapshot();
    changed.root = key(99);
    assert!(proof.verify(changed, key(20)).is_err());
    let mut changed = tree.snapshot();
    changed.context = IndexContext::new(key(1), key(4), 10).unwrap();
    assert!(proof.verify(changed, key(20)).is_err());
    let mut encoded = proof.to_bytes().unwrap();
    let end = encoded.len() - 1;
    encoded[end] ^= 1;
    assert!(
        IndexProof::from_bytes(&encoded)
            .unwrap()
            .verify(tree.snapshot(), key(20))
            .is_err()
    );
}
#[test]
fn duplicate_transactions_preserve_first_inclusion_and_batch_errors_are_atomic() {
    let mut tree = index();
    tree.append_block(11, &[key(20), key(20)]).unwrap();
    tree.append_block(12, &[key(20), key(90)]).unwrap();
    assert_eq!(
        tree.prove(key(20))
            .verify(tree.snapshot(), key(20))
            .unwrap(),
        Observation::Present(11)
    );
    let before = tree.snapshot();
    assert!(tree.append_block(13, &[key(30), Digest([0; 32])]).is_err());
    assert_eq!(tree.snapshot(), before);
    assert!(tree.append_block(14, &[key(40)]).is_err());
    assert_eq!(tree.snapshot(), before);
}
#[test]
fn full_256_bit_keys_distinguish_adjacent_ids() {
    let mut tree = index();
    let mut nearby = key(20);
    nearby.0[31] ^= 1;
    tree.append_block(11, &[key(20)]).unwrap();
    assert_eq!(
        tree.prove(nearby).verify(tree.snapshot(), nearby).unwrap(),
        Observation::Absent
    );
    let mut first_bit = key(20);
    first_bit.0[0] ^= 128;
    tree.append_block(12, &[nearby, first_bit]).unwrap();
    for txid in [nearby, first_bit] {
        assert_eq!(
            tree.prove(txid).verify(tree.snapshot(), txid).unwrap(),
            Observation::Present(12)
        );
    }
}
#[test]
fn collapsed_subtree_and_partial_paths_are_not_encodings() {
    let tree = index();
    let encoded = tree.prove(key(20)).to_bytes().unwrap();
    for len in [0, 32, 64, 128, encoded.len() - 1] {
        assert!(IndexProof::from_bytes(&encoded[..len]).is_err());
    }
    let mut trailing = encoded;
    trailing.push(0);
    assert!(IndexProof::from_bytes(&trailing).is_err());
}
#[test]
fn valid_candidate_hash_path_never_becomes_consensus_evidence() {
    let mut tree = index();
    tree.append_block(11, &[key(20)]).unwrap();
    assert!(tree.prove(key(21)).verify(tree.snapshot(), key(21)).is_ok());
    assert!(matches!(
        verify_history(tree.snapshot(), b"operator-signed root"),
        Err(Error::ConsensusUnavailable)
    ));
}
