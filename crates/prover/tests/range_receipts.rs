#![allow(clippy::unwrap_used)]
use zcash_protocol::consensus::NetworkType;
use zutti_protocol::{
    Digest,
    encoding::Canonical,
    statement::{ProofKind, Statement},
};
use zutti_prover::{artifact::ProofArtifact, remote::import_groth16};
use zutti_source::{
    proof::{SourceInput, SourcePolicy, SourceReceiptClaim, SourceWitness},
    range::{SourceRangeClaim, SourceRangeWitness},
};
fn import(fixture: &str, kind: ProofKind) -> ProofArtifact {
    let value: serde_json::Value = serde_json::from_str(fixture).unwrap();
    let payload = hex::decode(value["journal"].as_str().unwrap()).unwrap();
    let program = Digest::from_hex(value["image_id"].as_str().unwrap()).unwrap();
    let domain = if kind == ProofKind::SourceRange {
        SourceRangeClaim::from_bytes(&payload).unwrap().start.policy
    } else {
        SourceReceiptClaim::from_bytes(&payload).unwrap().policy
    };
    let statement = Statement::new(kind, domain, program, payload).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("result.json");
    zutti_wallet::backup::persist_new(&path, fixture.as_bytes()).unwrap();
    import_groth16(&path, &statement).unwrap()
}
#[test]
fn genuine_parallel_ranges_aggregate_and_fold_into_the_native_genesis_prefix() {
    let genesis = import(
        include_str!("../../../tests/fixtures/source-ranges-genesis-proof.json"),
        ProofKind::SourceTransition,
    );
    let a = import(
        include_str!("../../../tests/fixtures/source-ranges-0-2-proof.json"),
        ProofKind::SourceRange,
    );
    let b = import(
        include_str!("../../../tests/fixtures/source-ranges-2-4-proof.json"),
        ProofKind::SourceRange,
    );
    let aggregate = import(
        include_str!("../../../tests/fixtures/source-ranges-0-4-proof.json"),
        ProofKind::SourceRange,
    );
    let prefix = import(
        include_str!("../../../tests/fixtures/source-ranges-prefix-4-proof.json"),
        ProofKind::SourceTransition,
    );
    let initial = SourceReceiptClaim::from_bytes(genesis.statement().payload()).unwrap();
    let policy = SourcePolicy {
        program: initial.program,
        registry_program: Digest::from_hex(
            "f5b14f1fa25a45a6ae86316006e802d7adcf1df087ff8f4779da01922c1dc37f",
        )
        .unwrap(),
        network: NetworkType::Test,
        index_start: 0,
        confirmations: 3,
    };
    let witness = SourceWitness {
        policy,
        input: SourceInput::Genesis(zebra_test::vectors::TESTNET_BLOCKS[&0].to_vec()),
    };
    assert_eq!(initial, witness.evaluate().unwrap());
    let claims = [&a, &b].map(|p| SourceRangeClaim::from_bytes(p.statement().payload()).unwrap());
    let expected = SourceRangeWitness::Aggregate {
        policy,
        ranges: claims.to_vec(),
    }
    .evaluate()
    .unwrap();
    assert_eq!(
        expected,
        SourceRangeClaim::from_bytes(aggregate.statement().payload()).unwrap()
    );
    let folded = SourceWitness {
        policy,
        input: SourceInput::Fold {
            previous: Box::new(initial),
            range: Box::new(expected),
        },
    }
    .evaluate()
    .unwrap();
    assert_eq!(
        folded,
        SourceReceiptClaim::from_bytes(prefix.statement().payload()).unwrap()
    );
    assert_eq!(folded.height, 4);
    let mut candidate = zutti_source::state::SourceCandidate::from_genesis(
        &zutti_source::bootstrap::GenesisBlock::parse(
            zebra_test::vectors::TESTNET_BLOCKS[&0],
            NetworkType::Test,
        )
        .unwrap(),
        policy.id().unwrap(),
        0,
    )
    .unwrap();
    for (claim, (first, last)) in claims.iter().zip([(1, 2), (3, 4)]) {
        let transition = candidate
            .record_transition(
                (first..=last)
                    .map(|h| zebra_test::vectors::TESTNET_BLOCKS[&h].to_vec())
                    .collect(),
            )
            .unwrap();
        assert_eq!(
            *claim,
            SourceRangeWitness::transition(policy, transition)
                .unwrap()
                .evaluate()
                .unwrap()
        );
    }
    assert_eq!(
        folded.state,
        candidate.state_snapshot().unwrap().commitment().unwrap()
    );
    let wrong_kind = Statement::new(
        ProofKind::SourceTransition,
        a.statement().domain(),
        a.statement().program(),
        a.statement().payload().to_vec(),
    )
    .unwrap();
    assert!(a.verify(&wrong_kind).is_err());
    let mut bytes = prefix.to_bytes().unwrap();
    let end = bytes.len() - 1;
    bytes[end] ^= 1;
    assert!(
        ProofArtifact::from_bytes(&bytes).map_or(true, |p| p.verify(prefix.statement()).is_err())
    );
}
