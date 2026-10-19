#![allow(clippy::unwrap_used)]

use zcash_protocol::consensus::NetworkType;
use zutti_protocol::{
    Digest,
    encoding::Canonical,
    statement::{ProofKind, Statement},
};
use zutti_prover::{artifact::ProofArtifact, remote::import_groth16};
use zutti_source::proof::{SourceInput, SourcePolicy, SourceReceiptClaim, SourceWitness};
use zutti_source::range::SourceRangeClaim;

fn import(value: &serde_json::Value, kind: ProofKind, domain: Digest) -> ProofArtifact {
    let payload = hex::decode(value["journal"].as_str().unwrap()).unwrap();
    let program = Digest::from_hex(value["image_id"].as_str().unwrap()).unwrap();
    let expected = Statement::new(kind, domain, program, payload).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("result.json");
    zutti_wallet::backup::persist_new(&path, &serde_json::to_vec(value).unwrap()).unwrap();
    import_groth16(&path, &expected).unwrap()
}

fn genesis() -> (serde_json::Value, SourceReceiptClaim, SourcePolicy) {
    let value: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/source-optimized-genesis-proof.json"
    ))
    .unwrap();
    let claim =
        SourceReceiptClaim::from_bytes(&hex::decode(value["journal"].as_str().unwrap()).unwrap())
            .unwrap();
    let policy = SourcePolicy {
        program: claim.program,
        registry_program: Digest::from_hex(
            "6d89225b110c7548c11382263ba1941cb8b6d28c49d2a8c087cbbdc0694d7411",
        )
        .unwrap(),
        network: NetworkType::Test,
        index_start: 0,
        confirmations: 3,
    };
    (value, claim, policy)
}

#[test]
fn optimized_genesis_receipt_matches_native_validation_and_rejects_tampering() {
    let (value, claim, policy) = genesis();
    let proof = import(&value, ProofKind::SourceTransition, claim.policy);
    proof.verify(proof.statement()).unwrap();
    let expected = SourceWitness {
        policy,
        input: SourceInput::Genesis(zebra_test::vectors::TESTNET_BLOCKS[&0].to_vec()),
    }
    .evaluate()
    .unwrap();
    assert_eq!(claim, expected);
    let mut changed = value;
    let mut seal = hex::decode(changed["seal"].as_str().unwrap()).unwrap();
    seal[259] ^= 1;
    changed["seal"] = hex::encode(seal).into();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("result.json");
    zutti_wallet::backup::persist_new(&path, &serde_json::to_vec(&changed).unwrap()).unwrap();
    assert!(import_groth16(&path, proof.statement()).is_err());
}

#[test]
fn optimized_range_receipt_authenticates_the_genesis_to_128_boundary() {
    let (_, initial, policy) = genesis();
    let value: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/source-optimized-range-128-proof.json"
    ))
    .unwrap();
    let proof = import(&value, ProofKind::SourceRange, initial.policy);
    let range = SourceRangeClaim::from_bytes(proof.statement().payload()).unwrap();
    assert_eq!(range.start, initial);
    assert_eq!(range.end.height, 128);
    assert_eq!(range.end.program, policy.program);
    let folded = SourceWitness {
        policy,
        input: SourceInput::Fold {
            previous: Box::new(initial.clone()),
            range: Box::new(range.clone()),
        },
    }
    .evaluate()
    .unwrap();
    assert_eq!(folded, range.end);
    let mut changed = range;
    changed.start.state.0[0] ^= 1;
    assert!(
        SourceWitness {
            policy,
            input: SourceInput::Fold {
                previous: Box::new(initial),
                range: Box::new(changed),
            },
        }
        .evaluate()
        .is_err()
    );
}

#[test]
fn optimized_full_prefix_receipt_matches_the_proved_range_end() {
    let (_, initial, policy) = genesis();
    let range = serde_json::from_str(include_str!(
        "../../../tests/fixtures/source-optimized-range-128-proof.json"
    ))
    .unwrap();
    let range = import(&range, ProofKind::SourceRange, initial.policy);
    let range = SourceRangeClaim::from_bytes(range.statement().payload()).unwrap();
    let value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/source-optimized-prefix-128-proof.json"
    ))
    .unwrap();
    let prefix = import(&value, ProofKind::SourceTransition, initial.policy);
    let expected = SourceWitness {
        policy,
        input: SourceInput::Fold {
            previous: Box::new(initial),
            range: Box::new(range),
        },
    }
    .evaluate()
    .unwrap();
    assert_eq!(
        SourceReceiptClaim::from_bytes(prefix.statement().payload()).unwrap(),
        expected
    );
    assert_eq!(expected.height, 128);
}

#[test]
fn direct_prefix_receipt_matches_the_range_fold_result_and_rejects_tampering() {
    let (_, initial, _) = genesis();
    let value: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/source-optimized-direct-prefix-128-proof.json"
    ))
    .unwrap();
    let direct = import(&value, ProofKind::SourceTransition, initial.policy);
    let folded = serde_json::from_str(include_str!(
        "../../../tests/fixtures/source-optimized-prefix-128-proof.json"
    ))
    .unwrap();
    let folded = import(&folded, ProofKind::SourceTransition, initial.policy);
    assert_eq!(direct.statement(), folded.statement());
    let claim = SourceReceiptClaim::from_bytes(direct.statement().payload()).unwrap();
    assert_eq!(claim.height, 128);
    assert_eq!(claim.genesis, initial.genesis);
    let mut changed = value;
    let mut seal = hex::decode(changed["seal"].as_str().unwrap()).unwrap();
    seal[259] ^= 1;
    changed["seal"] = hex::encode(seal).into();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("result.json");
    zutti_wallet::backup::persist_new(&path, &serde_json::to_vec(&changed).unwrap()).unwrap();
    assert!(import_groth16(&path, direct.statement()).is_err());
}

#[test]
#[ignore = "requires retained optimized ELFs, public Testnet blocks and the genuine prefix artifact"]
fn optimized_prefix_completes_and_reopens_the_128_block_native_queue() {
    use std::path::PathBuf;
    use zutti_prover::{bootstrap::SourceBootstrap, source::SourcePrograms};
    use zutti_source::storage::RangeLimits;

    let programs = SourcePrograms::read(&PathBuf::from(
        std::env::var_os("ZUTTI_OPTIMIZED_PROGRAMS").unwrap(),
    ))
    .unwrap();
    let blocks = PathBuf::from(std::env::var_os("ZUTTI_TESTNET_BLOCKS").unwrap());
    let prefix = ProofArtifact::read(&PathBuf::from(
        std::env::var_os("ZUTTI_SOURCE_PREFIX").unwrap(),
    ))
    .unwrap();
    let (value, initial, policy) = genesis();
    assert_eq!(policy, programs.policy(NetworkType::Test, 0, 3));
    let genesis = import(&value, ProofKind::SourceTransition, initial.policy);
    let value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/source-optimized-range-128-proof.json"
    ))
    .unwrap();
    let range = import(&value, ProofKind::SourceRange, initial.policy);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.sqlite");
    let limits = RangeLimits::default();
    let mut bootstrap = SourceBootstrap::create(
        &path,
        std::fs::read(blocks.join("0.bin")).unwrap(),
        policy,
        programs.clone(),
        limits,
    )
    .unwrap();
    bootstrap.accept_prefix(&genesis).unwrap();
    let raw = (1..=128)
        .map(|height| std::fs::read(blocks.join(format!("{height}.bin"))).unwrap())
        .collect();
    let batch = bootstrap.advance(raw).unwrap();
    assert_eq!(
        bootstrap.range_job(batch).unwrap().statement(),
        range.statement()
    );
    assert_eq!(
        bootstrap.continue_job(batch).unwrap().statement(),
        prefix.statement()
    );
    bootstrap.accept_prefix(&prefix).unwrap();
    assert!(bootstrap.store().batches().is_empty());
    drop(bootstrap);
    let restored = SourceBootstrap::open(&path, policy, programs, limits).unwrap();
    assert_eq!(restored.store().base().height, 128);
    assert_eq!(restored.store().candidate().height().unwrap(), 128);
    assert!(restored.store().batches().is_empty());
    let claim = SourceReceiptClaim::from_bytes(prefix.statement().payload()).unwrap();
    assert_eq!(
        restored
            .store()
            .candidate()
            .state_snapshot()
            .unwrap()
            .commitment()
            .unwrap(),
        claim.state
    );
}
