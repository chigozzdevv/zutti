#![allow(clippy::unwrap_used)]
use zcash_protocol::consensus::NetworkType;
use zutti_protocol::{Digest, encoding::Canonical};
use zutti_source::{
    bootstrap::GenesisBlock,
    proof::{SourceInput, SourcePolicy, SourceReceiptClaim, SourceWitness},
    range::{MAX_RANGE_INPUTS, SourceRangeClaim, SourceRangeWitness},
    state::SourceCandidate,
};

fn fixtures() -> (SourcePolicy, SourceReceiptClaim, Vec<SourceRangeWitness>) {
    let policy = SourcePolicy {
        program: Digest([1; 32]),
        registry_program: Digest([2; 32]),
        network: NetworkType::Test,
        index_start: 0,
        confirmations: 3,
    };
    let bytes = zebra_test::vectors::TESTNET_BLOCKS[&0].to_vec();
    let genesis = SourceWitness {
        policy,
        input: SourceInput::Genesis(bytes.clone()),
    }
    .evaluate()
    .unwrap();
    let block = GenesisBlock::parse(&bytes, policy.network).unwrap();
    let mut source = SourceCandidate::from_genesis(&block, policy.id().unwrap(), 0).unwrap();
    let mut ranges = Vec::new();
    for first in [1_u32, 3, 5, 7] {
        let transition = source
            .record_transition(
                (first..first + 2)
                    .map(|h| zebra_test::vectors::TESTNET_BLOCKS[&h].to_vec())
                    .collect(),
            )
            .unwrap();
        ranges.push(SourceRangeWitness::transition(policy, transition).unwrap());
    }
    (policy, genesis, ranges)
}

#[test]
fn independent_ranges_join_at_exact_authenticated_states() {
    let (policy, genesis, witnesses) = fixtures();
    let ranges: Vec<_> = witnesses.iter().map(|w| w.evaluate().unwrap()).collect();
    assert_eq!(ranges[0].start, genesis);
    for pair in ranges.windows(2) {
        assert_eq!(pair[0].end, pair[1].start);
    }
    let whole = SourceRangeClaim::aggregate(policy, &ranges).unwrap();
    assert_eq!(whole.start.height, 0);
    assert_eq!(whole.end.height, 8);
    let left = SourceRangeClaim::aggregate(policy, &ranges[..2]).unwrap();
    let right = SourceRangeClaim::aggregate(policy, &ranges[2..]).unwrap();
    assert_eq!(
        SourceRangeClaim::aggregate(policy, &[left, right]).unwrap(),
        whole
    );
    let folded = SourceWitness {
        policy,
        input: SourceInput::Fold {
            previous: Box::new(genesis),
            range: Box::new(whole.clone()),
        },
    };
    assert_eq!(folded.evaluate().unwrap(), whole.end);
    assert_eq!(
        SourceWitness::from_bytes(&folded.to_bytes().unwrap())
            .unwrap()
            .evaluate()
            .unwrap(),
        whole.end
    );
    assert!(SourceReceiptClaim::from_bytes(&whole.to_bytes().unwrap()).is_err());
}

#[test]
fn gaps_reordering_wrong_policies_and_changed_boundaries_fail() {
    let (policy, genesis, witnesses) = fixtures();
    let ranges: Vec<_> = witnesses.iter().map(|w| w.evaluate().unwrap()).collect();
    for bad in [
        vec![ranges[0].clone(), ranges[2].clone()],
        vec![ranges[1].clone(), ranges[0].clone()],
        vec![ranges[0].clone(), ranges[0].clone()],
    ] {
        assert!(SourceRangeClaim::aggregate(policy, &bad).is_err());
    }
    let mut changed = policy;
    changed.network = NetworkType::Main;
    assert!(SourceRangeClaim::aggregate(changed, &ranges).is_err());
    let mut whole = SourceRangeClaim::aggregate(policy, &ranges).unwrap();
    whole.start.state.0[0] ^= 1;
    let folded = SourceWitness {
        policy,
        input: SourceInput::Fold {
            previous: Box::new(genesis),
            range: Box::new(whole),
        },
    };
    assert!(folded.evaluate().is_err());
    assert!(SourceRangeClaim::new(ranges[0].end.clone(), ranges[0].start.clone()).is_err());
    assert!(SourceRangeClaim::new(ranges[0].start.clone(), ranges[0].start.clone()).is_err());
}

#[test]
fn range_witnesses_are_bounded_and_do_not_accept_genesis_as_a_transition() {
    let (policy, _, witnesses) = fixtures();
    for witness in &witnesses {
        let bytes = witness.to_bytes().unwrap();
        assert_eq!(
            SourceRangeWitness::from_bytes(&bytes)
                .unwrap()
                .evaluate()
                .unwrap(),
            witness.evaluate().unwrap()
        );
        for end in [0, 8, bytes.len() - 1] {
            assert!(SourceRangeWitness::from_bytes(&bytes[..end]).is_err());
        }
        let mut extra = bytes;
        extra.push(0);
        assert!(SourceRangeWitness::from_bytes(&extra).is_err());
    }
    let claim = witnesses[0].evaluate().unwrap();
    for count in [0, 1, MAX_RANGE_INPUTS + 1] {
        let witness = SourceRangeWitness::Aggregate {
            policy,
            ranges: vec![claim.clone(); count],
        };
        assert!(witness.to_bytes().is_err());
        assert!(witness.evaluate().is_err());
    }
    let witness = SourceRangeWitness::Transition(Box::new(SourceWitness {
        policy,
        input: SourceInput::Genesis(zebra_test::vectors::TESTNET_BLOCKS[&0].to_vec()),
    }));
    assert!(witness.to_bytes().is_err());
    assert!(witness.evaluate().is_err());
}
