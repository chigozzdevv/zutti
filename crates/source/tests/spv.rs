#![allow(clippy::unwrap_used)]

use zcash_protocol::consensus::{NetworkType, TEST_NETWORK};
use zutti_protocol::{Digest, Secret32, encoding::Canonical};
use zutti_source::{
    block::ParsedBlock,
    proof::{SourcePolicy, SourceReceiptClaim},
    spv::{
        Branch, Error, HEADER_BYTES, HeaderClaim, Policy, WindowClaim, WindowTerms, WindowWitness,
        Witness, parse_header,
    },
};

fn raw(height: u32) -> Vec<u8> {
    hex::decode(
        match height {
            0 => include_str!("../../../tests/fixtures/block-test-0.txt"),
            1 => include_str!("../../../tests/fixtures/spv/testnet-1.hex"),
            2 => include_str!("../../../tests/fixtures/spv/testnet-2.hex"),
            3 => include_str!("../../../tests/fixtures/spv/testnet-3.hex"),
            4 => include_str!("../../../tests/fixtures/spv/testnet-4.hex"),
            _ => panic!("unknown fixture"),
        }
        .trim(),
    )
    .unwrap()
}

fn policy() -> Policy {
    Policy::new(Digest([1; 32]), NetworkType::Test, 1, 3, 4).unwrap()
}
fn prefix() -> HeaderClaim {
    HeaderClaim::genesis(policy(), &raw(0)[..HEADER_BYTES]).unwrap()
}
fn terms() -> WindowTerms {
    WindowTerms::new(
        Digest([2; 32]),
        Digest([3; 32]),
        0,
        prefix().tip().unwrap(),
        2,
        Digest([4; 32]),
        Digest([5; 32]),
    )
    .unwrap()
}
fn target(height: u32) -> Digest {
    ParsedBlock::parse(&raw(height), &TEST_NETWORK)
        .unwrap()
        .txids()[0]
}
fn query(txid: Digest, branch: Branch, bodies: &[u32], tip: u32) -> WindowWitness {
    WindowWitness::new(
        terms(),
        txid,
        Secret32::from_bytes([6; 32]).unwrap(),
        branch,
        (1..=tip).map(|h| raw(h)[..HEADER_BYTES].to_vec()).collect(),
        bodies.iter().map(|h| raw(*h)).collect(),
    )
    .unwrap()
}

#[test]
fn header_prefix_is_genesis_linked_and_immutable_on_rejection() {
    let start = prefix();
    let before = start.to_bytes().unwrap();
    let end = start
        .advance(
            &(1..=4)
                .map(|h| raw(h)[..HEADER_BYTES].to_vec())
                .collect::<Vec<_>>(),
        )
        .unwrap();
    assert_eq!(end.height().unwrap(), 4);
    assert_eq!(
        end.tip().unwrap(),
        Digest(parse_header(&raw(4)[..HEADER_BYTES]).unwrap().hash().0)
    );
    assert!(end.work().unwrap() > start.work().unwrap());
    assert!(start.advance(&[raw(2)[..HEADER_BYTES].to_vec()]).is_err());
    assert_eq!(start.to_bytes().unwrap(), before);
    assert!(
        HeaderClaim::genesis(
            policy(),
            &hex::decode(include_str!("../../../tests/fixtures/block-0.txt").trim()).unwrap()
                [..HEADER_BYTES]
        )
        .is_err()
    );
    let mut changed = before.clone();
    changed.push(0);
    assert!(HeaderClaim::from_bytes(&changed).is_err());
    assert_eq!(
        HeaderClaim::from_bytes(&before)
            .unwrap()
            .to_bytes()
            .unwrap(),
        before
    );
}

#[test]
fn modified_header_fields_and_noncanonical_solution_lengths_fail() {
    let first = raw(1)[..HEADER_BYTES].to_vec();
    for offset in [0, 4, 36, 100, 104, 108, 143, HEADER_BYTES - 1] {
        let mut altered = first.clone();
        altered[offset] ^= 1;
        assert!(prefix().advance(&[altered]).is_err(), "offset {offset}");
    }
    for length in [0, 140, 143, HEADER_BYTES - 1] {
        assert!(parse_header(&first[..length]).is_err());
    }
    let mut altered = first.clone();
    altered.push(0);
    assert!(parse_header(&altered).is_err());
    let mut altered = first;
    altered[141] ^= 1;
    assert!(parse_header(&altered).is_err());
}

#[test]
fn inclusion_at_expiry_stays_payable_when_claim_is_delayed() {
    let txid = target(2);
    let early = query(txid, Branch::Included, &[2], 3)
        .evaluate(&prefix())
        .unwrap();
    let late = query(txid, Branch::Included, &[2], 4)
        .evaluate(&prefix())
        .unwrap();
    assert_eq!(early.output, terms().payout());
    assert_eq!(late.output, early.output);
    assert_eq!(late.reservation, early.reservation);
    assert!(late.height > early.height);
    let unpaid = query(txid, Branch::Absent, &[1, 2], 4);
    assert_eq!(unpaid.reservation(policy()).unwrap(), early.reservation);
    assert_eq!(unpaid.evaluate(&prefix()), Err(Error::Outcome));
}

#[test]
fn a_refund_candidate_needs_complete_absence_and_a_confirmed_expiry_boundary() {
    let absent = Digest([77; 32]);
    let result = query(absent, Branch::Absent, &[1, 2], 3)
        .evaluate(&prefix())
        .unwrap();
    assert_eq!(result.output, terms().refund());
    assert_eq!(
        WindowClaim::from_bytes(&result.to_bytes().unwrap()).unwrap(),
        result
    );
    assert_eq!(
        query(absent, Branch::Absent, &[1, 2], 2).evaluate(&prefix()),
        Err(Error::Confirmations)
    );
    for bodies in [&[1][..], &[2][..], &[2, 1][..], &[1, 1][..], &[1, 3][..]] {
        assert!(
            query(absent, Branch::Absent, bodies, 3)
                .evaluate(&prefix())
                .is_err()
        );
    }
    assert_eq!(
        query(absent, Branch::Included, &[1], 3).evaluate(&prefix()),
        Err(Error::Outcome)
    );
    assert!(
        query(target(3), Branch::Included, &[3], 4)
            .evaluate(&prefix())
            .is_err()
    );
}

#[test]
fn early_inclusion_does_not_wait_for_expiry() {
    let claim = query(target(1), Branch::Included, &[1], 2)
        .evaluate(&prefix())
        .unwrap();
    assert_eq!(claim.height, 2);
    assert_eq!(claim.output, terms().payout());
    assert_eq!(
        query(target(1), Branch::Included, &[1], 1).evaluate(&prefix()),
        Err(Error::Confirmations)
    );
}

#[test]
fn omitted_or_fabricated_block_data_cannot_create_absence() {
    let mut missing = raw(2);
    missing[HEADER_BYTES] = 0;
    missing.truncate(HEADER_BYTES + 1);
    let mut witness = query(target(2), Branch::Absent, &[1, 2], 3)
        .to_bytes()
        .unwrap();
    let original = raw(2);
    let position = witness
        .windows(original.len())
        .position(|part| part == original)
        .unwrap();
    witness.splice(
        position - 4..position + original.len(),
        (missing.len() as u32)
            .to_le_bytes()
            .into_iter()
            .chain(missing),
    );
    assert!(
        WindowWitness::from_bytes(&witness)
            .unwrap()
            .evaluate(&prefix())
            .is_err()
    );
    let mut modified = raw(2);
    let last = modified.len() - 1;
    modified[last] ^= 1;
    assert!(ParsedBlock::parse(&modified, &TEST_NETWORK).is_err());
}

#[test]
fn policy_window_and_full_validity_encodings_are_separate() {
    assert!(SourcePolicy::from_bytes(&policy().to_bytes().unwrap()).is_err());
    assert!(SourceReceiptClaim::from_bytes(&prefix().to_bytes().unwrap()).is_err());
    let witness = Witness::Window {
        previous: prefix(),
        window: query(Digest([77; 32]), Branch::Absent, &[1, 2], 3),
    };
    let bytes = witness.to_bytes().unwrap();
    assert_eq!(
        Witness::from_bytes(&bytes)
            .unwrap()
            .evaluate()
            .unwrap()
            .journal()
            .unwrap(),
        witness.evaluate().unwrap().journal().unwrap()
    );
    assert!(zutti_source::proof::SourceWitness::from_bytes(&bytes).is_err());
    let mut trailing = bytes;
    trailing.push(0);
    assert!(Witness::from_bytes(&trailing).is_err());
    assert!(Policy::new(Digest([1; 32]), NetworkType::Regtest, 1, 3, 4).is_err());
    assert!(Policy::new(Digest([1; 32]), NetworkType::Test, 0, 3, 4).is_err());
    assert!(Policy::new(Digest([1; 32]), NetworkType::Test, 1, 129, 4).is_err());
    let changed = Policy::new(Digest([9; 32]), NetworkType::Test, 1, 3, 4).unwrap();
    assert_ne!(changed.id().unwrap(), policy().id().unwrap());
    assert_ne!(
        query(target(2), Branch::Included, &[2], 3)
            .reservation(changed)
            .unwrap(),
        query(target(2), Branch::Included, &[2], 3)
            .reservation(policy())
            .unwrap()
    );
}

#[test]
fn public_window_journals_expose_neither_txid_nor_branch_selector() {
    let txid = target(2);
    let claim = query(txid, Branch::Included, &[2], 3)
        .evaluate(&prefix())
        .unwrap();
    let bytes = claim.to_bytes().unwrap();
    assert!(!bytes.windows(32).any(|part| part == txid.0));
    assert_eq!(claim.output, terms().payout());
    assert!(zutti_source::query::QueryClaim::from_bytes(&bytes).is_err());
}

#[test]
fn headers_stop_at_the_explicit_policy_horizon() {
    let policy = Policy::new(Digest([1; 32]), NetworkType::Test, 1, 1, 1).unwrap();
    let start = HeaderClaim::genesis(policy, &raw(0)[..HEADER_BYTES]).unwrap();
    let first = start.advance(&[raw(1)[..HEADER_BYTES].to_vec()]).unwrap();
    assert_eq!(first.height().unwrap(), 1);
    assert!(matches!(
        first.advance(&[raw(2)[..HEADER_BYTES].to_vec()]),
        Err(Error::Policy)
    ));
    assert!(
        query(Digest([77; 32]), Branch::Absent, &[1, 2], 3)
            .evaluate(&first)
            .is_err()
    );
}

#[test]
fn an_initial_header_batch_is_bound_to_genesis_without_an_unresolved_predecessor() {
    let witness = Witness::Genesis {
        policy: policy(),
        header: raw(0)[..HEADER_BYTES].to_vec(),
        headers: vec![raw(1)[..HEADER_BYTES].to_vec()],
    };
    assert!(witness.assumption().is_none());
    let expected = prefix()
        .advance(&[raw(1)[..HEADER_BYTES].to_vec()])
        .unwrap()
        .to_bytes()
        .unwrap();
    assert_eq!(witness.evaluate().unwrap().journal().unwrap(), expected);
    assert_eq!(
        Witness::from_bytes(&witness.to_bytes().unwrap())
            .unwrap()
            .evaluate()
            .unwrap()
            .journal()
            .unwrap(),
        expected
    );
    let wrong = Witness::Genesis {
        policy: policy(),
        header: raw(1)[..HEADER_BYTES].to_vec(),
        headers: vec![],
    };
    assert!(wrong.evaluate().is_err());
}

#[test]
fn legacy_64_byte_transactions_are_rejected_before_merkle_shape_can_be_ambiguous() {
    use sha2::{Digest as _, Sha256};
    use zcash_encoding::CompactSize;
    use zcash_primitives::{block::BlockHeaderData, transaction::Transaction};
    use zcash_protocol::consensus::BranchId;
    let block = ParsedBlock::parse(&raw(1), &TEST_NETWORK).unwrap();
    let mut encoded = 1_u32.to_le_bytes().to_vec();
    encoded.extend_from_slice(&[0, 1]);
    encoded.extend_from_slice(&0_u64.to_le_bytes());
    encoded.push(45);
    encoded.extend_from_slice(&[0; 45]);
    encoded.extend_from_slice(&0_u32.to_le_bytes());
    assert_eq!(encoded.len(), 64);
    let tx = Transaction::read(encoded.as_slice(), BranchId::Sprout).unwrap();
    let mut leaves = Vec::new();
    leaves.extend_from_slice(&block.txids()[0].0);
    leaves.extend_from_slice(tx.txid().as_ref());
    let merkle_root: [u8; 32] = Sha256::digest(Sha256::digest(&leaves)).into();
    let old = block.header();
    let header = BlockHeaderData {
        version: old.version,
        prev_block: old.prev_block,
        merkle_root,
        final_sapling_root: old.final_sapling_root,
        time: old.time,
        bits: old.bits,
        nonce: old.nonce,
        solution: old.solution.clone(),
    }
    .freeze()
    .unwrap();
    let mut raw = Vec::new();
    header.write(&mut raw).unwrap();
    CompactSize::write(&mut raw, 2).unwrap();
    block.transactions()[0].write(&mut raw).unwrap();
    raw.extend_from_slice(&encoded);
    assert!(ParsedBlock::parse(&raw, &TEST_NETWORK).is_ok());
    assert!(matches!(
        zutti_source::spv::parse_block(&raw, NetworkType::Test),
        Err(Error::AmbiguousMerkleLeaf)
    ));
}
