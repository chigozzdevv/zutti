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

#[test]
fn longer_checkpoint_windows_allow_payments_without_expanding_absence_searches() {
    assert!(Policy::new(Digest([1; 32]), NetworkType::Test, 10, 129, 5_000_000).is_err());
    let paid =
        Policy::from_checkpoint(Digest([1; 32]), 10, 512, 5_000_000, Digest([2; 32])).unwrap();
    assert!(paid.paid_only());
    assert_eq!(Policy::from_bytes(&paid.to_bytes().unwrap()).unwrap(), paid);
    assert!(Policy::from_checkpoint(Digest([1; 32]), 10, 513, 5_000_000, Digest([2; 32])).is_err());
    assert_eq!(zutti_source::spv::MAX_WINDOW_BLOCKS, 128);
    let mut legacy = paid.to_bytes().unwrap();
    legacy[8..10].copy_from_slice(&1_u16.to_le_bytes());
    legacy.truncate(legacy.len() - 32);
    assert!(Policy::from_bytes(&legacy).is_err());
}

#[test]
fn checkpoint_origin_binds_the_complete_context_and_rejects_genesis_substitution() {
    let original = HeaderClaim::genesis(policy(), &raw(0)[..HEADER_BYTES])
        .unwrap()
        .advance(&[
            raw(1)[..HEADER_BYTES].to_vec(),
            raw(2)[..HEADER_BYTES].to_vec(),
        ])
        .unwrap();
    let pinned =
        Policy::from_checkpoint(Digest([2; 32]), 1, 3, 4, original.checkpoint_id().unwrap())
            .unwrap();
    assert!(pinned.paid_only());
    assert_ne!(policy().id().unwrap(), pinned.id().unwrap());
    assert_eq!(
        Policy::from_bytes(&pinned.to_bytes().unwrap()).unwrap(),
        pinned
    );
    assert!(HeaderClaim::genesis(pinned, &raw(0)[..HEADER_BYTES]).is_err());
    let witness = Witness::Checkpoint {
        policy: pinned,
        checkpoint: original.clone(),
        headers: vec![raw(3)[..HEADER_BYTES].to_vec()],
    };
    assert!(witness.required_journal().unwrap().is_none());
    let bytes = witness.to_bytes().unwrap();
    let journal = Witness::from_bytes(&bytes)
        .unwrap()
        .evaluate()
        .unwrap()
        .journal()
        .unwrap();
    assert_eq!(
        HeaderClaim::from_bytes(&journal).unwrap().height().unwrap(),
        3
    );
    let changed = original
        .advance(&[raw(3)[..HEADER_BYTES].to_vec()])
        .unwrap();
    assert!(HeaderClaim::from_checkpoint(pinned, &changed).is_err());
    assert!(Policy::from_checkpoint(Digest([2; 32]), 1, 3, 4, Digest::default()).is_err());
    let mut changed_time = original.to_bytes().unwrap();
    *changed_time.last_mut().unwrap() ^= 1;
    if let Ok(changed) = HeaderClaim::from_bytes(&changed_time) {
        assert!(HeaderClaim::from_checkpoint(pinned, &changed).is_err());
    }
}

#[test]
fn checkpoint_context_requires_linked_real_headers_and_sufficient_work() {
    let context: Vec<_> = (0..=2)
        .map(|height| raw(height)[..HEADER_BYTES].to_vec())
        .collect();
    let original = HeaderClaim::genesis(policy(), &context[0])
        .unwrap()
        .advance(&context[1..])
        .unwrap();
    let checkpoint =
        HeaderClaim::checkpoint_context(policy(), 2, &context, original.work().unwrap()).unwrap();
    assert_eq!(
        checkpoint.checkpoint_id().unwrap(),
        original.checkpoint_id().unwrap()
    );
    assert!(
        HeaderClaim::checkpoint_context(policy(), 2, &context[1..], original.work().unwrap())
            .is_err()
    );
    assert!(HeaderClaim::checkpoint_context(policy(), 2, &context, [0; 32]).is_err());
    let mut broken = context.clone();
    broken.swap(1, 2);
    assert!(
        HeaderClaim::checkpoint_context(policy(), 2, &broken, original.work().unwrap()).is_err()
    );
}

#[test]
fn checkpoint_payment_window_rejects_absence_without_treating_it_as_a_refund() {
    let original = HeaderClaim::genesis(policy(), &raw(0)[..HEADER_BYTES])
        .unwrap()
        .advance(&[raw(1)[..HEADER_BYTES].to_vec()])
        .unwrap();
    let pinned =
        Policy::from_checkpoint(Digest([2; 32]), 1, 2, 4, original.checkpoint_id().unwrap())
            .unwrap();
    let anchor = HeaderClaim::from_checkpoint(pinned, &original).unwrap();
    let terms = WindowTerms::new(
        Digest([3; 32]),
        Digest([4; 32]),
        1,
        anchor.tip().unwrap(),
        3,
        Digest([5; 32]),
        Digest([6; 32]),
    )
    .unwrap();
    let witness = WindowWitness::new(
        terms,
        Digest([7; 32]),
        Secret32::from_bytes([8; 32]).unwrap(),
        Branch::Absent,
        (2..=4)
            .map(|height| raw(height)[..HEADER_BYTES].to_vec())
            .collect(),
        vec![raw(2), raw(3)],
    )
    .unwrap();
    assert!(matches!(witness.evaluate(&anchor), Err(Error::Outcome)));
}

#[test]
fn shielded_transaction_bodies_are_covered_by_the_actual_block_root() {
    use zcash_protocol::consensus::MAIN_NETWORK;
    use zutti_source::spv::parse_block;
    let raw =
        hex::decode(include_str!("../../../tests/fixtures/block-1687107.txt").trim()).unwrap();
    let block = parse_block(&raw, NetworkType::Main).unwrap();
    assert_eq!(block.claimed_height(), 1_687_107);
    assert_eq!(block.transactions().len(), 6);
    let index = block
        .transactions()
        .iter()
        .position(|tx| tx.orchard_bundle().is_some())
        .unwrap();
    assert_ne!(index, 0);
    let tx = &block.transactions()[index];
    assert!(!tx.transparent_bundle().is_some_and(|b| b.is_coinbase()));
    assert!(block.txids().contains(&Digest(*tx.txid().as_ref())));
    assert_eq!(
        ParsedBlock::parse(&raw, &MAIN_NETWORK).unwrap().hash(),
        block.hash()
    );
    let mut changed = raw.clone();
    changed.pop();
    assert!(parse_block(&changed, NetworkType::Main).is_err());
    let mut encoded = Vec::new();
    block.header().write(&mut encoded).unwrap();
    zcash_encoding::CompactSize::write(&mut encoded, block.transactions().len() - 1).unwrap();
    for (position, transaction) in block.transactions().iter().enumerate() {
        if position != index {
            transaction.write(&mut encoded).unwrap();
        }
    }
    assert!(parse_block(&encoded, NetworkType::Main).is_err());
}

#[test]
fn dense_testnet_sapling_block_cannot_omit_a_shielded_transaction() {
    use zutti_source::{spv::parse_block, work::verify_header_work};
    let raw = include_bytes!("../../../tests/fixtures/spv/testnet-282757.bin");
    let block = parse_block(raw, NetworkType::Test).unwrap();
    verify_header_work(block.header(), NetworkType::Test).unwrap();
    assert_eq!(block.claimed_height(), 282_757);
    assert_eq!(block.transactions().len(), 1635);
    assert_eq!(
        block
            .transactions()
            .iter()
            .skip(1)
            .filter(|tx| tx.sapling_bundle().is_some())
            .count(),
        1634
    );
    let target = block.txids()[1];
    assert_eq!(Digest(*block.transactions()[1].txid().as_ref()), target);
    let mut omitted = Vec::new();
    block.header().write(&mut omitted).unwrap();
    zcash_encoding::CompactSize::write(&mut omitted, block.transactions().len() - 1).unwrap();
    for (position, transaction) in block.transactions().iter().enumerate() {
        if position != 1 {
            transaction.write(&mut omitted).unwrap();
        }
    }
    assert!(parse_block(&omitted, NetworkType::Test).is_err());
}

#[test]
fn admission_binds_the_stable_prefix_exact_depth_and_next_work() {
    use zutti_source::spv::{AdmissionClaim, AdmissionKind, AdmissionWitness};
    let witness = AdmissionWitness::Advance {
        previous: prefix(),
        stable_headers: (1..=3).map(|h| raw(h)[..HEADER_BYTES].to_vec()).collect(),
        confirmations: vec![raw(4)[..HEADER_BYTES].to_vec()],
    };
    let claim = witness.evaluate().unwrap();
    assert_eq!(claim.kind, AdmissionKind::Advance);
    assert_eq!(claim.previous, Digest::default());
    assert_eq!(claim.stable.height, 3);
    assert_eq!(claim.head.height, 4);
    assert!(claim.head.work > claim.stable.work);
    assert_eq!(
        claim.head.prefix,
        prefix()
            .advance(
                &(1..=4)
                    .map(|h| raw(h)[..HEADER_BYTES].to_vec())
                    .collect::<Vec<_>>()
            )
            .unwrap()
            .id()
            .unwrap()
    );
    assert_eq!(
        AdmissionClaim::from_bytes(&claim.to_bytes().unwrap()).unwrap(),
        claim
    );
    assert_eq!(
        AdmissionWitness::from_bytes(&witness.to_bytes().unwrap())
            .unwrap()
            .evaluate()
            .unwrap(),
        claim
    );
    for confirmations in [
        vec![],
        vec![
            raw(3)[..HEADER_BYTES].to_vec(),
            raw(4)[..HEADER_BYTES].to_vec(),
        ],
    ] {
        assert!(
            AdmissionWitness::Advance {
                previous: prefix(),
                stable_headers: vec![
                    raw(1)[..HEADER_BYTES].to_vec(),
                    raw(2)[..HEADER_BYTES].to_vec()
                ],
                confirmations
            }
            .evaluate()
            .is_err()
        );
    }
    let previous = prefix()
        .advance(&[raw(1)[..HEADER_BYTES].to_vec()])
        .unwrap();
    let update = AdmissionWitness::Advance {
        previous: previous.clone(),
        stable_headers: vec![raw(2)[..HEADER_BYTES].to_vec()],
        confirmations: vec![raw(3)[..HEADER_BYTES].to_vec()],
    }
    .evaluate()
    .unwrap();
    assert_eq!(update.previous, previous.id().unwrap());
}

#[test]
fn initialization_can_admit_an_already_proved_prefix_without_a_recent_trust_anchor() {
    use zutti_source::spv::{AdmissionClaim, AdmissionWitness};
    let stable = prefix()
        .advance(
            &(1..=3)
                .map(|h| raw(h)[..HEADER_BYTES].to_vec())
                .collect::<Vec<_>>(),
        )
        .unwrap();
    let witness = AdmissionWitness::Initialize {
        stable: stable.clone(),
        confirmations: vec![raw(4)[..HEADER_BYTES].to_vec()],
    };
    let claim = witness.evaluate().unwrap();
    assert_eq!(claim.previous, Digest::default());
    assert_eq!(claim.stable.prefix, stable.id().unwrap());
    assert_eq!(claim.stable.height, 3);
    assert_eq!(claim.head.height, 4);
    assert_eq!(
        witness.predecessor().to_bytes().unwrap(),
        stable.to_bytes().unwrap()
    );
    assert_eq!(
        AdmissionWitness::from_bytes(&witness.to_bytes().unwrap())
            .unwrap()
            .evaluate()
            .unwrap(),
        claim
    );
    assert_eq!(
        AdmissionClaim::from_bytes(&claim.to_bytes().unwrap()).unwrap(),
        claim
    );
}

#[test]
fn initialization_rejects_genesis_missing_confirmations_and_wrong_ancestry() {
    use zutti_source::spv::AdmissionWitness;
    let stable = prefix()
        .advance(&[raw(1)[..HEADER_BYTES].to_vec()])
        .unwrap();
    for (stable, confirmations) in [
        (prefix(), vec![raw(1)[..HEADER_BYTES].to_vec()]),
        (stable.clone(), vec![]),
        (stable.clone(), vec![raw(3)[..HEADER_BYTES].to_vec()]),
        (
            stable,
            vec![
                raw(2)[..HEADER_BYTES].to_vec(),
                raw(3)[..HEADER_BYTES].to_vec(),
            ],
        ),
    ] {
        assert!(
            AdmissionWitness::Initialize {
                stable,
                confirmations
            }
            .evaluate()
            .is_err()
        );
    }
}

#[test]
fn conflicts_need_a_different_stable_prefix_and_confirmed_alternative() {
    use zutti_source::spv::{AdmissionKind, AdmissionWitness};
    let alternative = prefix()
        .advance(&[raw(1)[..HEADER_BYTES].to_vec()])
        .unwrap();
    assert!(
        AdmissionWitness::Conflict {
            alternative: alternative.clone(),
            accepted_stable: alternative.id().unwrap(),
            headers: vec![raw(2)[..HEADER_BYTES].to_vec()]
        }
        .evaluate()
        .is_err()
    );
    let conflict = AdmissionWitness::Conflict {
        alternative: alternative.clone(),
        accepted_stable: Digest([8; 32]),
        headers: vec![raw(2)[..HEADER_BYTES].to_vec()],
    }
    .evaluate()
    .unwrap();
    assert_eq!(conflict.kind, AdmissionKind::Conflict);
    assert_eq!(conflict.stable.prefix, alternative.id().unwrap());
    assert!(
        AdmissionWitness::Conflict {
            alternative,
            accepted_stable: Digest([8; 32]),
            headers: vec![]
        }
        .evaluate()
        .is_err()
    );
}

#[test]
fn prefix_timestamp_ceiling_covers_every_appended_header() {
    let headers = (1..=4)
        .map(|h| raw(h)[..HEADER_BYTES].to_vec())
        .collect::<Vec<_>>();
    let claim = prefix().advance(&headers).unwrap();
    let expected = (0..=4)
        .map(|h| parse_header(&raw(h)[..HEADER_BYTES]).unwrap().time)
        .max()
        .unwrap();
    assert_eq!(claim.maximum_time(), expected);
    let roundtrip = HeaderClaim::from_bytes(&claim.to_bytes().unwrap()).unwrap();
    assert_eq!(roundtrip.maximum_time(), expected);
    let observed = query(Digest([77; 32]), Branch::Absent, &[1, 2], 4)
        .evaluate(&prefix())
        .unwrap();
    assert_eq!(observed.checkpoint, claim.id().unwrap());
    let mut legacy = claim.to_bytes().unwrap();
    legacy[8..10].copy_from_slice(&1_u16.to_le_bytes());
    assert!(HeaderClaim::from_bytes(&legacy).is_err());
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
        window: Box::new(query(Digest([77; 32]), Branch::Absent, &[1, 2], 3)),
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
    assert!(witness.required_journal().unwrap().is_none());
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
