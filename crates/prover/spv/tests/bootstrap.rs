use bincode::Options;
use risc0_zkvm::{FakeReceipt, InnerReceipt, Receipt, ReceiptClaim};
use std::{
    fs,
    path::{Path, PathBuf},
};
use zcash_protocol::consensus::NetworkType;
use zutti_protocol::encoding::Canonical;
use zutti_source::spv::{HEADER_BYTES, HeaderClaim, Policy, Witness};
use zutti_spv_prototype::{
    bootstrap::{Bootstrap, headers},
    image,
};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/genesis-coverage")
}

fn genesis() -> Vec<u8> {
    hex::decode(include_str!("../../../../tests/fixtures/block-test-0.txt").trim()).unwrap()
        [..HEADER_BYTES]
        .to_vec()
}

fn first() -> Vec<u8> {
    hex::decode(include_str!("../../../../tests/fixtures/spv/testnet-1.hex").trim()).unwrap()
        [..HEADER_BYTES]
        .to_vec()
}

fn setup(root: &Path) -> Bootstrap {
    let program = fs::read(fixture().join("program.elf")).unwrap();
    let policy = Policy::new(image(&program).unwrap(), NetworkType::Test, 1, 3, 4).unwrap();
    Bootstrap::init(root, &program, &genesis(), policy).unwrap()
}

#[test]
fn checkpoint_bootstrap_survives_restart_and_never_invents_a_receipt() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("checkpoint-state");
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/boundless-source");
    let program = fs::read(directory.join("program.elf")).unwrap();
    let checkpoint =
        HeaderClaim::from_bytes(&fs::read(directory.join("journal-0.bin")).unwrap()).unwrap();
    let policy = Policy::from_checkpoint(
        image(&program).unwrap(),
        1,
        3,
        10,
        checkpoint.checkpoint_id().unwrap(),
    )
    .unwrap();
    let state = Bootstrap::init_checkpoint(&root, &program, &checkpoint, policy).unwrap();
    let batch = state.prepare(vec![]).unwrap();
    let status = state.status().unwrap();
    assert_eq!(status.bootstrap, "checkpoint");
    assert_eq!(status.checkpoint_height, Some(2));
    assert_eq!(status.verified_height, None);
    assert_eq!(status.pending_height, Some(2));
    assert!(state.accept(0, &directory.join("receipt-0.bin")).is_err());
    assert!(!batch.join("receipt.bin").exists());
    drop(state);
    let state = Bootstrap::open(&root).unwrap();
    assert_eq!(state.status().unwrap().pending_height, Some(2));
    drop(state);
    let path = root.join("checkpoint.bin");
    let mut bytes = fs::read(&path).unwrap();
    bytes[0] ^= 1;
    fs::write(path, bytes).unwrap();
    assert!(Bootstrap::open(&root).is_err());
}

#[test]
fn genuine_receipt_advances_once_and_restart_preserves_its_exact_predecessor() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("state");
    let state = setup(&root);
    let batch = state.prepare(vec![]).unwrap();
    let pending = state.status().unwrap();
    assert_eq!(pending.verified_height, None);
    assert_eq!(pending.pending_height, Some(0));
    assert_eq!(state.prepare(vec![]).unwrap(), batch);
    let status = state.accept(0, &fixture().join("receipt.bin")).unwrap();
    assert_eq!(status.verified_height, Some(0));
    assert_eq!(status.verified_batches, 1);
    assert!(!status.canonicality_admitted);
    assert!(!status.settlement_authority);
    assert_eq!(
        state
            .accept(0, &fixture().join("receipt.bin"))
            .unwrap()
            .verified_batches,
        1
    );
    drop(state);
    let state = Bootstrap::open(&root).unwrap();
    assert_eq!(state.status().unwrap().verified_height, Some(0));
    let next = state.prepare(vec![first()]).unwrap();
    let witness = Witness::from_bytes(&fs::read(next.join("witness.bin")).unwrap()).unwrap();
    assert_eq!(
        witness.required_journal().unwrap().unwrap(),
        fs::read(fixture().join("journal.bin")).unwrap()
    );
    assert!(state.accept(1, &fixture().join("receipt.bin")).is_err());
    assert!(!next.join("receipt.bin").exists());
    let status = state.status().unwrap();
    assert_eq!(status.verified_height, Some(0));
    assert_eq!(status.pending_height, Some(1));
}

#[test]
fn direct_genesis_batch_prepares_real_headers_without_inventing_verified_progress() {
    let temp = tempfile::tempdir().unwrap();
    let state = setup(&temp.path().join("state"));
    state.prepare(vec![first()]).unwrap();
    let status = state.status().unwrap();
    assert_eq!(status.verified_height, None);
    assert_eq!(status.pending_height, Some(1));
    assert!(state.prepare(vec![]).is_err());
    assert!(state.accept(0, &fixture().join("receipt.bin")).is_err());
    assert_eq!(state.status().unwrap().verified_batches, 0);
}

#[test]
fn invalid_and_reordered_headers_never_publish_a_job() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("state");
    let state = setup(&root);
    let mut changed = first();
    changed[4] ^= 1;
    assert!(state.prepare(vec![changed]).is_err());
    assert!(state.prepare(vec![first(), first()]).is_err());
    assert_eq!(fs::read_dir(root.join("jobs")).unwrap().count(), 0);
}

#[test]
fn matching_fake_receipt_is_rejected_without_advancing_or_persisting_it() {
    let temp = tempfile::tempdir().unwrap();
    let state = setup(&temp.path().join("state"));
    let batch = state.prepare(vec![]).unwrap();
    let journal = fs::read(batch.join("journal.bin")).unwrap();
    let claim = HeaderClaim::from_bytes(&journal).unwrap();
    let fake = Receipt::new(
        InnerReceipt::Fake(FakeReceipt::new(ReceiptClaim::ok(
            claim.policy().program().0,
            journal.clone(),
        ))),
        journal,
    );
    let path = temp.path().join("fake.bin");
    fs::write(
        &path,
        bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .serialize(&fake)
            .unwrap(),
    )
    .unwrap();
    assert!(state.accept(0, &path).is_err());
    assert!(!batch.join("receipt.bin").exists());
    assert_eq!(state.status().unwrap().verified_batches, 0);
}

#[test]
fn genuine_historical_program_receipt_cannot_advance_the_current_image() {
    let temp = tempfile::tempdir().unwrap();
    let state = setup(&temp.path().join("state"));
    let batch = state.prepare(vec![]).unwrap();
    let historical =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/genesis-v2/receipt.bin");
    assert!(state.accept(0, &historical).is_err());
    assert!(!batch.join("receipt.bin").exists());
    assert_eq!(state.status().unwrap().verified_batches, 0);
}

#[test]
fn restart_reverifies_completed_receipts_and_rejects_changed_seals() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("state");
    let state = setup(&root);
    let batch = state.prepare(vec![]).unwrap();
    state.accept(0, &fixture().join("receipt.bin")).unwrap();
    drop(state);
    let path = batch.join("receipt.bin");
    let mut receipt = zutti_spv_prototype::read_receipt(&path).unwrap();
    let InnerReceipt::Composite(inner) = &mut receipt.inner else {
        panic!("expected genuine composite");
    };
    inner.segments[0].seal[0] ^= 1;
    fs::write(
        path,
        bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .serialize(&receipt)
            .unwrap(),
    )
    .unwrap();
    let state = Bootstrap::open(&root).unwrap();
    assert!(state.status().is_err());
    assert!(state.prepare(vec![first()]).is_err());
}

#[test]
fn incomplete_staging_is_ignored_but_missing_jobs_and_changed_artifacts_fail_closed() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("state");
    let state = setup(&root);
    let stage = root.join("jobs/.spv-prepare-interrupted");
    fs::create_dir(&stage).unwrap();
    fs::write(stage.join("witness.bin"), b"incomplete").unwrap();
    assert_eq!(state.status().unwrap().verified_batches, 0);
    let batch = state.prepare(vec![]).unwrap();
    let path = batch.join("batch.json");
    let original = fs::read(&path).unwrap();
    let mut manifest: serde_json::Value = serde_json::from_slice(&original).unwrap();
    manifest["last_height"] = 1.into();
    fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(state.status().is_err());
    fs::write(path, original).unwrap();
    fs::rename(batch, root.join("jobs/00000001")).unwrap();
    assert!(state.status().is_err());
}

#[test]
fn state_lock_prevents_concurrent_mutations_and_releases_on_drop() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("state");
    let state = setup(&root);
    assert!(Bootstrap::open(&root).is_err());
    drop(state);
    Bootstrap::open(&root).unwrap();
}

#[test]
fn configuration_change_and_truncated_header_import_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("state");
    drop(setup(&root));
    let path = temp.path().join("headers.bin");
    fs::write(&path, first()).unwrap();
    assert_eq!(headers(&path).unwrap(), vec![first()]);
    fs::write(&path, &first()[..HEADER_BYTES - 1]).unwrap();
    assert!(headers(&path).is_err());
    let config = root.join("config.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    value["program_sha256"] = "changed".into();
    fs::write(config, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(Bootstrap::open(&root).is_err());
}

#[cfg(unix)]
#[test]
fn symlinked_state_and_receipts_are_rejected() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("state");
    let state = setup(&root);
    let batch = state.prepare(vec![]).unwrap();
    let alias = temp.path().join("alias");
    symlink(&root, &alias).unwrap();
    assert!(Bootstrap::open(&alias).is_err());
    symlink(fixture().join("receipt.bin"), batch.join("receipt.bin")).unwrap();
    assert!(state.status().is_err());
}
