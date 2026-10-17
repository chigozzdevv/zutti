#![allow(clippy::unwrap_used)]
use zutti_protocol::{
    Digest,
    encoding::Canonical,
    statement::{ProofKind, Statement},
};
use zutti_prover::{Error, ProofEnvelope, ProofVerifier};
fn statement(kind: ProofKind, n: u8) -> Statement {
    Statement::new(kind, Digest([1; 32]), Digest([n; 32]), vec![2; 64]).unwrap()
}
#[test]
fn arbitrary_receipts_and_mock_proofs_are_never_admitted() {
    let expected = statement(ProofKind::Source, 3);
    for bytes in [
        b"valid".to_vec(),
        vec![0; 128],
        b"risc0-development-receipt".to_vec(),
    ] {
        let proof = ProofEnvelope::new(&expected, bytes).unwrap();
        assert!(matches!(
            ProofVerifier.verify(&proof, &expected),
            Err(Error::BackendUnavailable)
        ));
    }
}
#[test]
fn proof_kind_program_domain_and_payload_are_bound() {
    let expected = statement(ProofKind::Source, 3);
    let proof = ProofEnvelope::new(&expected, vec![1]).unwrap();
    for changed in [
        statement(ProofKind::Settlement, 3),
        statement(ProofKind::Source, 4),
        Statement::new(
            ProofKind::Source,
            Digest([9; 32]),
            Digest([3; 32]),
            vec![2; 64],
        )
        .unwrap(),
        Statement::new(
            ProofKind::Source,
            Digest([1; 32]),
            Digest([3; 32]),
            vec![3; 64],
        )
        .unwrap(),
    ] {
        assert!(matches!(
            ProofVerifier.verify(&proof, &changed),
            Err(Error::Binding)
        ));
    }
}
#[test]
fn malformed_proofs_are_bounded_and_rejected() {
    let expected = statement(ProofKind::Source, 3);
    assert!(ProofEnvelope::new(&expected, vec![]).is_err());
    let proof = ProofEnvelope::new(&expected, vec![1; 64]).unwrap();
    let bytes = proof.to_bytes().unwrap();
    assert_eq!(ProofEnvelope::from_bytes(&bytes).unwrap(), proof);
    for len in 0..bytes.len() {
        assert!(ProofEnvelope::from_bytes(&bytes[..len]).is_err());
    }
    let mut bad = bytes;
    bad.push(0);
    assert!(ProofEnvelope::from_bytes(&bad).is_err());
}
