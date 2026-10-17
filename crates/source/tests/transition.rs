#![cfg(feature = "blocks")]
#![allow(clippy::unwrap_used)]

use zcash_protocol::consensus::NetworkType;
use zutti_protocol::{Digest, encoding::Canonical};
use zutti_source::{
    bootstrap::GenesisBlock,
    state::SourceCandidate,
    transition::{StateSnapshot, TransitionWitness},
};

fn source(start: u32) -> SourceCandidate {
    let genesis =
        GenesisBlock::parse(zebra_test::vectors::MAINNET_BLOCKS[&0], NetworkType::Main).unwrap();
    SourceCandidate::from_genesis(&genesis, Digest([1; 32]), start).unwrap()
}
fn blocks(first: u32, last: u32) -> Vec<Vec<u8>> {
    (first..=last)
        .map(|h| zebra_test::vectors::MAINNET_BLOCKS[&h].to_vec())
        .collect()
}

#[test]
fn authenticated_transitions_match_stateful_validation_for_each_partition() {
    for start in [0, 5, 20] {
        let mut baseline = source(start);
        baseline.record_transition(blocks(1, 10)).unwrap();
        let expected = baseline.state_snapshot().unwrap().to_bytes().unwrap();
        for split in [1, 4, 9] {
            let mut state = source(start);
            for raw in [blocks(1, split), blocks(split + 1, 10)] {
                let before = state.state_snapshot().unwrap().commitment().unwrap();
                let witness = state.record_transition(raw).unwrap();
                let encoded = witness.to_bytes().unwrap();
                let decoded = TransitionWitness::from_bytes(&encoded).unwrap();
                assert_eq!(decoded.before().commitment().unwrap(), before);
                let after = decoded.evaluate().unwrap();
                assert_eq!(
                    after.to_bytes().unwrap(),
                    state.state_snapshot().unwrap().to_bytes().unwrap()
                );
                assert_eq!(
                    StateSnapshot::from_bytes(&after.to_bytes().unwrap())
                        .unwrap()
                        .commitment()
                        .unwrap(),
                    after.commitment().unwrap()
                );
            }
            assert_eq!(
                state.state_snapshot().unwrap().to_bytes().unwrap(),
                expected
            );
        }
    }
}

#[test]
fn forged_prior_state_bad_paths_extra_accesses_and_invalid_blocks_fail() {
    let mut state = source(0);
    let witness = state.record_transition(blocks(1, 2)).unwrap();
    let encoded = witness.to_bytes().unwrap();
    let before = witness.before().to_bytes().unwrap();
    let mut forged = encoded.clone();
    let position = before
        .windows(32)
        .position(|bytes| bytes == witness.before().index().root.0)
        .unwrap();
    forged[10 + position] ^= 1;
    assert!(
        TransitionWitness::from_bytes(&forged)
            .unwrap()
            .evaluate()
            .is_err()
    );
    let mut changed = encoded.clone();
    let position = changed
        .windows(8)
        .position(|bytes| bytes == b"ZUTTIPTH")
        .unwrap();
    changed[position + 10] ^= 1;
    assert!(
        TransitionWitness::from_bytes(&changed)
            .unwrap()
            .evaluate()
            .is_err()
    );
    let snapshot = state.state_snapshot().unwrap().commitment().unwrap();
    assert!(state.record_transition(blocks(4, 4)).is_err());
    assert_eq!(
        state.state_snapshot().unwrap().commitment().unwrap(),
        snapshot
    );
    let mut extra = encoded;
    extra.push(0);
    assert!(TransitionWitness::from_bytes(&extra).is_err());
}

#[test]
fn checkpoints_authenticate_the_complete_index_and_agreed_header_ancestry() {
    let mut state = source(0);
    state.record_transition(blocks(1, 10)).unwrap();
    let checkpoint = state.checkpoint(3).unwrap();
    assert_eq!(checkpoint.stable.height, 7);
    assert_eq!(checkpoint.stable.index.height, 7);
    let path = state.prove_header(1).unwrap();
    let header = path
        .verify(
            checkpoint.stable.index.context.genesis(),
            Digest([1; 32]),
            checkpoint.archive,
            1,
        )
        .unwrap();
    checkpoint.verify_anchor(1, header.hash, &path).unwrap();
    assert!(
        checkpoint
            .verify_anchor(1, Digest([99; 32]), &path)
            .is_err()
    );
    assert!(checkpoint.verify_anchor(2, header.hash, &path).is_err());
    assert!(checkpoint.verify_anchor(8, header.hash, &path).is_err());
    assert!(state.checkpoint(0).is_err());
    assert!(state.checkpoint(11).is_err());
    assert_ne!(
        checkpoint.id().unwrap(),
        state.checkpoint(2).unwrap().id().unwrap()
    );
}

#[test]
fn recursive_inputs_require_the_exact_policy_and_predecessor_state() {
    use zutti_source::proof::{SourceInput, SourcePolicy, SourceReceiptClaim, SourceWitness};
    let policy = SourcePolicy {
        program: Digest([22; 32]),
        registry_program: Digest([23; 32]),
        network: NetworkType::Main,
        index_start: 0,
        confirmations: 3,
    };
    let genesis_bytes = zebra_test::vectors::MAINNET_BLOCKS[&0].to_vec();
    let genesis = SourceWitness {
        policy,
        input: SourceInput::Genesis(genesis_bytes.clone()),
    };
    let claim = genesis.evaluate().unwrap();
    assert_eq!(claim.height, 0);
    assert!(genesis.assumption().is_none());
    assert_eq!(
        SourceReceiptClaim::from_bytes(&claim.to_bytes().unwrap()).unwrap(),
        claim
    );
    let genesis_block = GenesisBlock::parse(&genesis_bytes, NetworkType::Main).unwrap();
    let mut state = SourceCandidate::from_genesis(&genesis_block, policy.id().unwrap(), 0).unwrap();
    let transition = state.record_transition(blocks(1, 2)).unwrap();
    let advance = SourceWitness {
        policy,
        input: SourceInput::Advance {
            previous: Box::new(claim.clone()),
            transition: Box::new(transition.clone()),
        },
    };
    assert_eq!(advance.assumption(), Some(&claim));
    let output = SourceWitness::from_bytes(&advance.to_bytes().unwrap())
        .unwrap()
        .evaluate()
        .unwrap();
    assert_eq!(output.height, 2);
    assert_eq!(
        output.state,
        state.state_snapshot().unwrap().commitment().unwrap()
    );
    let mut wrong = claim;
    wrong.state.0[0] ^= 1;
    assert!(
        SourceWitness {
            policy,
            input: SourceInput::Advance {
                previous: Box::new(wrong),
                transition: Box::new(transition)
            }
        }
        .evaluate()
        .is_err()
    );
    let mut wrong_network = genesis.clone();
    wrong_network.policy.network = NetworkType::Test;
    assert!(wrong_network.evaluate().is_err());
    let mut bad_genesis = genesis;
    if let SourceInput::Genesis(bytes) = &mut bad_genesis.input {
        bytes[0] ^= 1;
    }
    assert!(bad_genesis.evaluate().is_err());
}

#[test]
fn source_updates_preserve_accepted_stable_ancestry_and_increasing_work() {
    use zutti_source::{
        proof::{SourceInput, SourcePolicy, SourceWitness},
        update::{PreviousUpdate, SourceUpdateWitness},
    };
    let policy = SourcePolicy {
        program: Digest([22; 32]),
        registry_program: Digest([23; 32]),
        network: NetworkType::Main,
        index_start: 0,
        confirmations: 3,
    };
    let bytes = zebra_test::vectors::MAINNET_BLOCKS[&0].to_vec();
    let genesis = GenesisBlock::parse(&bytes, NetworkType::Main).unwrap();
    let mut state = SourceCandidate::from_genesis(&genesis, policy.id().unwrap(), 0).unwrap();
    let first = SourceWitness {
        policy,
        input: SourceInput::Genesis(bytes),
    }
    .evaluate()
    .unwrap();
    let transition = state.record_transition(blocks(1, 7)).unwrap();
    let chain = SourceWitness {
        policy,
        input: SourceInput::Advance {
            previous: Box::new(first),
            transition: Box::new(transition),
        },
    }
    .evaluate()
    .unwrap();
    let checkpoint = state.checkpoint(3).unwrap();
    let initial = SourceUpdateWitness {
        policy,
        chain: chain.clone(),
        stable: state.prove_header(4).unwrap(),
        previous: None,
    };
    let claim = initial.evaluate().unwrap();
    assert_eq!(claim.previous, Digest::default());
    assert_eq!(claim.checkpoint, checkpoint.id().unwrap());
    let transition = state.record_transition(blocks(8, 10)).unwrap();
    let chain = SourceWitness {
        policy,
        input: SourceInput::Advance {
            previous: Box::new(chain),
            transition: Box::new(transition),
        },
    }
    .evaluate()
    .unwrap();
    let mut update = SourceUpdateWitness {
        policy,
        chain,
        stable: state.prove_header(7).unwrap(),
        previous: Some(Box::new(PreviousUpdate {
            claim: claim.clone(),
            checkpoint,
            ancestry: state.prove_header(4).unwrap(),
        })),
    };
    let next = SourceUpdateWitness::from_bytes(&update.to_bytes().unwrap())
        .unwrap()
        .evaluate()
        .unwrap();
    assert_eq!(next.previous, claim.state);
    assert_eq!(next.previous_stable, claim.checkpoint);
    assert!(next.work_be > claim.work_be);
    update.previous.as_mut().unwrap().claim.work_be = next.work_be;
    assert!(update.evaluate().is_err());
    update.previous.as_mut().unwrap().claim.work_be = claim.work_be;
    update.previous.as_mut().unwrap().ancestry = state.prove_header(5).unwrap();
    assert!(update.evaluate().is_err());
    update.previous.as_mut().unwrap().ancestry = state.prove_header(4).unwrap();
    update.previous.as_mut().unwrap().checkpoint.stable.hash.0[0] ^= 1;
    assert!(update.evaluate().is_err());
}
