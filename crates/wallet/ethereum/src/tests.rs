#![allow(clippy::unwrap_used)]

use super::*;
use alloy::primitives::Bytes;
use alloy::rpc::types::EIP1186StorageProof;
use alloy_trie::{HashBuilder, proof::ProofRetainer};
use helios_consensus_core::{consensus_spec::MinimalConsensusSpec, types::Fork};
use ssz::Decode;
use std::path::PathBuf;

type TestConsensus = (
    ConsensusConfig,
    Bootstrap<MinimalConsensusSpec>,
    FinalityUpdate<MinimalConsensusSpec>,
);

pub(super) fn fixture(fork: &str) -> TestConsensus {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(fork);
    let meta: serde_yaml::Value =
        serde_yaml::from_slice(&std::fs::read(path.join("meta.yaml")).unwrap()).unwrap();
    let steps: serde_yaml::Value =
        serde_yaml::from_slice(&std::fs::read(path.join("steps.yaml")).unwrap()).unwrap();
    let fork_at = |version: [u8; 4], epoch| Fork {
        epoch,
        fork_version: version.into(),
    };
    let config = ConsensusConfig {
        chain_id: 5,
        genesis_time: 1_000,
        genesis_root: meta["genesis_validators_root"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap(),
        checkpoint: meta["trusted_block_root"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap(),
        seconds_per_slot: 6,
        max_checkpoint_age: 1_209_600,
        max_finalized_age: 600,
        forks: Forks {
            genesis: fork_at([0, 0, 0, 1], 0),
            altair: fork_at([1, 0, 0, 1], 0),
            bellatrix: fork_at([2, 0, 0, 1], 0),
            capella: fork_at([3, 0, 0, 1], 0),
            deneb: fork_at([4, 0, 0, 1], 0),
            electra: fork_at([5, 0, 0, 1], if fork == "gloas" { 0 } else { u64::MAX }),
            fulu: fork_at([6, 0, 0, 1], if fork == "gloas" { 0 } else { u64::MAX }),
            gloas: fork_at([7, 0, 0, 1], if fork == "gloas" { 0 } else { u64::MAX }),
        },
    };
    let decode = |name: &str| {
        snap::raw::Decoder::new()
            .decompress_vec(&std::fs::read(path.join(name)).unwrap())
            .unwrap()
    };
    let bootstrap = Bootstrap::from_ssz_bytes(&decode("bootstrap.ssz_snappy")).unwrap();
    let update_name = steps[0]["process_update"]["update"].as_str().unwrap();
    let update = Update::<MinimalConsensusSpec>::from_ssz_bytes(&decode(&format!(
        "{update_name}.ssz_snappy"
    )))
    .unwrap();
    let finality = as_finality(update);
    (config, bootstrap, finality)
}

fn as_finality(update: Update<MinimalConsensusSpec>) -> FinalityUpdate<MinimalConsensusSpec> {
    let mut value = serde_json::to_value(update).unwrap();
    value.as_object_mut().unwrap().remove("next_sync_committee");
    value
        .as_object_mut()
        .unwrap()
        .remove("next_sync_committee_branch");
    serde_json::from_value(value).unwrap()
}

#[test]
fn official_consensus_vectors_authenticate_finality_across_forks() {
    for fork in ["deneb", "gloas"] {
        let (config, bootstrap, finality) = fixture(fork);
        let header = verify_consensus(&config, &bootstrap, &[], &finality, 1_246).unwrap();
        assert_eq!(header.beacon().slot, 24);
        assert!(!execution_hash(&header).unwrap().is_zero());
    }
}

#[test]
fn rejects_modified_consensus_and_stale_or_future_evidence() {
    for fork in ["deneb", "gloas"] {
        let (mut config, bootstrap, finality) = fixture(fork);
        config.checkpoint[0] ^= 1;
        assert!(verify_consensus(&config, &bootstrap, &[], &finality, 1_246).is_err());
        config.checkpoint[0] ^= 1;
        config.genesis_root[0] ^= 1;
        assert!(verify_consensus(&config, &bootstrap, &[], &finality, 1_246).is_err());
        config.genesis_root[0] ^= 1;
        assert!(verify_consensus(&config, &bootstrap, &[], &finality, 1_240).is_err());
        assert!(verify_consensus(&config, &bootstrap, &[], &finality, 2_000_000).is_err());
        config.max_finalized_age = 90;
        assert!(verify_consensus(&config, &bootstrap, &[], &finality, 1_246).is_err());
        config.max_finalized_age = 600;
        let mut invalid = serde_json::to_value(&finality).unwrap();
        invalid["sync_aggregate"]["sync_committee_signature"] =
            serde_json::Value::String(format!("0x{}", "00".repeat(96)));
        let invalid = serde_json::from_value(invalid).unwrap();
        assert!(verify_consensus(&config, &bootstrap, &[], &invalid, 1_246).is_err());
    }
}

fn trie(leaves: Vec<(B256, Vec<u8>)>, keys: &[B256]) -> (B256, Vec<Vec<Bytes>>) {
    let targets: Vec<_> = keys
        .iter()
        .map(|key| Nibbles::unpack(keccak256(key)))
        .collect();
    let mut builder =
        HashBuilder::default().with_proof_retainer(ProofRetainer::new(targets.clone()));
    let mut leaves: Vec<_> = leaves
        .into_iter()
        .map(|(key, value)| (Nibbles::unpack(keccak256(key)), value))
        .collect();
    leaves.sort_unstable_by_key(|entry| entry.0);
    for (key, value) in leaves {
        builder.add_leaf(key, &value);
    }
    let root = builder.root();
    let nodes = builder.take_proof_nodes();
    let proofs = targets
        .iter()
        .map(|target| {
            nodes
                .matching_nodes_sorted(target)
                .into_iter()
                .map(|(_, value)| value)
                .collect()
        })
        .collect();
    (root, proofs)
}

fn reserved() -> (ReservationRequest, EIP1186AccountProofResponse, B256) {
    let expected = ReservationRequest {
        chain_id: 11155111,
        pool: Address::repeat_byte(1),
        pool_code_hash: B256::repeat_byte(2),
        escrow: B256::repeat_byte(3),
        order_tag: B256::repeat_byte(4),
        nullifier: B256::repeat_byte(5),
    };
    let keys = [
        mapping_key(expected.escrow, 70),
        mapping_key(expected.order_tag, 71),
        mapping_key(expected.nullifier, 69),
    ];
    let (storage_root, proofs) = trie(
        vec![
            (keys[0], rlp::encode(U256::from(1))),
            (keys[1], rlp::encode(U256::from(1))),
        ],
        &keys,
    );
    let account = TrieAccount {
        nonce: 1,
        balance: U256::ZERO,
        storage_root,
        code_hash: expected.pool_code_hash,
    };
    let account_key = Nibbles::unpack(keccak256(expected.pool));
    let mut builder =
        HashBuilder::default().with_proof_retainer(ProofRetainer::new(vec![account_key]));
    builder.add_leaf(account_key, &rlp::encode(account));
    let root = builder.root();
    let nodes = builder.take_proof_nodes();
    let proof = EIP1186AccountProofResponse {
        address: expected.pool,
        nonce: 1,
        balance: U256::ZERO,
        storage_hash: storage_root,
        code_hash: expected.pool_code_hash,
        account_proof: nodes
            .matching_nodes_sorted(&account_key)
            .into_iter()
            .map(|(_, value)| value)
            .collect(),
        storage_proof: keys
            .into_iter()
            .zip(proofs)
            .enumerate()
            .map(|(index, (key, proof))| EIP1186StorageProof {
                key: key.into(),
                value: if index == 2 {
                    U256::ZERO
                } else {
                    U256::from(1)
                },
                proof,
            })
            .collect(),
    };
    (expected, proof, root)
}

#[test]
fn verifies_inclusion_and_non_membership_at_the_same_authenticated_root() {
    let (expected, proof, root) = reserved();
    verify_reserved_storage(&expected, &proof, root).unwrap();
    let mut wrong = root;
    wrong[0] ^= 1;
    assert!(verify_reserved_storage(&expected, &proof, wrong).is_err());
    for change in 0..7 {
        let mut altered = proof.clone();
        match change {
            0 => altered.code_hash[0] ^= 1,
            1 => altered.address[0] ^= 1,
            2 => altered.storage_proof[0].value = U256::ZERO,
            3 => altered.storage_proof[2].value = U256::from(1),
            4 => altered.storage_proof[2] = altered.storage_proof[0].clone(),
            5 => {
                altered.storage_proof.pop();
            }
            _ => altered.storage_proof[0].proof.clear(),
        }
        assert!(
            verify_reserved_storage(&expected, &altered, root).is_err(),
            "accepted mutation {change}"
        );
    }
}

#[test]
fn verifies_committee_rotation_and_rejects_missing_predecessors() {
    for fork in ["deneb", "gloas"] {
        let (config, bootstrap, _) = fixture(fork);
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(fork);
        let steps: serde_yaml::Value =
            serde_yaml::from_slice(&std::fs::read(path.join("steps.yaml")).unwrap()).unwrap();
        let decode = |index: usize| {
            let name = steps[index]["process_update"]["update"].as_str().unwrap();
            snap::raw::Decoder::new()
                .decompress_vec(&std::fs::read(path.join(format!("{name}.ssz_snappy"))).unwrap())
                .unwrap()
        };
        let update = Update::<MinimalConsensusSpec>::from_ssz_bytes(&decode(0)).unwrap();
        let finality =
            as_finality(Update::<MinimalConsensusSpec>::from_ssz_bytes(&decode(1)).unwrap());
        let header = verify_consensus(&config, &bootstrap, &[update], &finality, 1_534).unwrap();
        assert_eq!(header.beacon().slot, 72);
        assert!(verify_consensus(&config, &bootstrap, &[], &finality, 1_534).is_err());
    }
}

#[test]
fn finalized_execution_hash_binds_the_complete_header_and_timestamp() {
    let (config, bootstrap, finality) = fixture("gloas");
    let mut finalized = verify_consensus(&config, &bootstrap, &[], &finality, 1_246).unwrap();
    let execution = Header {
        timestamp: 1_144,
        state_root: B256::repeat_byte(8),
        number: 42,
        ..Default::default()
    };
    let LightClientHeader::Gloas(header) = &mut finalized else {
        panic!("wrong fixture fork")
    };
    header.execution_block_hash = execution.hash_slow();
    check_execution(&finalized, &execution, 600, 1_246).unwrap();
    let mut altered = execution.clone();
    altered.state_root[0] ^= 1;
    assert!(check_execution(&finalized, &altered, 600, 1_246).is_err());
    assert!(check_execution(&finalized, &execution, 600, 1_140).is_err());
    assert!(check_execution(&finalized, &execution, 90, 1_246).is_err());
}

#[test]
fn finalized_descendants_authenticate_captured_state_without_archive_rpc_trust() {
    let (config, bootstrap, finality) = fixture("gloas");
    let mut finalized = verify_consensus(&config, &bootstrap, &[], &finality, 1_246).unwrap();
    let captured = Header {
        number: 42,
        timestamp: 1_120,
        state_root: B256::repeat_byte(8),
        ..Default::default()
    };
    let child = Header {
        number: 43,
        timestamp: 1_132,
        parent_hash: captured.hash_slow(),
        ..Default::default()
    };
    let tip = Header {
        number: 44,
        timestamp: 1_144,
        parent_hash: child.hash_slow(),
        ..Default::default()
    };
    let LightClientHeader::Gloas(header) = &mut finalized else {
        panic!("wrong fork")
    };
    header.execution_block_hash = tip.hash_slow();
    let chain = vec![child, tip];
    check_execution_path(&finalized, &captured, &chain, 600, 1_246).unwrap();
    for index in 0..chain.len() {
        for field in ["parent", "number", "timestamp", "state"] {
            let mut altered = chain.clone();
            match field {
                "parent" => altered[index].parent_hash[0] ^= 1,
                "number" => altered[index].number += 1,
                "timestamp" => altered[index].timestamp = captured.timestamp,
                _ => altered[index].state_root[0] ^= 1,
            }
            assert!(check_execution_path(&finalized, &captured, &altered, 600, 1_246).is_err());
        }
    }
    assert!(check_execution_path(&finalized, &captured, &chain[1..], 600, 1_246).is_err());
    assert!(check_execution_path(&finalized, &captured, &chain, 100, 1_246).is_err());
    let mut wrong = captured;
    wrong.state_root[0] ^= 1;
    assert!(check_execution_path(&finalized, &wrong, &chain, 600, 1_246).is_err());
}

#[test]
fn note_state_requires_a_known_root_and_a_genuinely_unspent_nullifier() {
    use crate::notes::{NoteRequest, verify_note_storage};
    let request = NoteRequest {
        chain_id: 11155111,
        pool: Address::repeat_byte(1),
        pool_code_hash: B256::repeat_byte(2),
        root: B256::repeat_byte(3),
        nullifier: B256::repeat_byte(4),
    };
    let keys = request.storage_keys().unwrap();
    for spent in [false, true] {
        let mut leaves = vec![(keys[0], rlp::encode(U256::from(1)))];
        if spent {
            leaves.push((keys[1], rlp::encode(U256::from(1))));
        }
        let (storage_root, proofs) = trie(leaves, &keys);
        let account = TrieAccount {
            nonce: 1,
            balance: U256::ZERO,
            storage_root,
            code_hash: request.pool_code_hash,
        };
        let account_key = Nibbles::unpack(keccak256(request.pool));
        let mut builder =
            HashBuilder::default().with_proof_retainer(ProofRetainer::new(vec![account_key]));
        builder.add_leaf(account_key, &rlp::encode(account));
        let root = builder.root();
        let proof = EIP1186AccountProofResponse {
            address: request.pool,
            nonce: 1,
            balance: U256::ZERO,
            storage_hash: storage_root,
            code_hash: request.pool_code_hash,
            account_proof: builder
                .take_proof_nodes()
                .matching_nodes_sorted(&account_key)
                .into_iter()
                .map(|(_, value)| value)
                .collect(),
            storage_proof: keys
                .into_iter()
                .zip(proofs)
                .enumerate()
                .map(|(i, (key, proof))| EIP1186StorageProof {
                    key: key.into(),
                    value: U256::from(u64::from(i == 0 || spent)),
                    proof,
                })
                .collect(),
        };
        assert_eq!(verify_note_storage(&request, &proof, root).is_ok(), !spent);
        if !spent {
            for change in [
                "root",
                "nullifier",
                "code",
                "pool",
                "missing",
                "duplicate",
                "value",
            ] {
                let mut request = request.clone();
                let mut proof = proof.clone();
                match change {
                    "root" => request.root[0] ^= 1,
                    "nullifier" => request.nullifier[0] ^= 1,
                    "code" => request.pool_code_hash[0] ^= 1,
                    "pool" => request.pool[0] ^= 1,
                    "missing" => {
                        proof.storage_proof.pop();
                    }
                    "duplicate" => proof.storage_proof[1] = proof.storage_proof[0].clone(),
                    _ => proof.storage_proof[1].value = U256::from(1),
                }
                assert!(
                    verify_note_storage(&request, &proof, root).is_err(),
                    "accepted {change}"
                );
            }
        }
    }
}

#[test]
fn retained_live_sepolia_note_evidence_authenticates_the_actual_deployed_pool() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../deploy/ethereum");
    let config: ConsensusConfig =
        serde_json::from_slice(&std::fs::read(path.join("sepolia-consensus.json")).unwrap())
            .unwrap();
    let request: crate::notes::NoteRequest =
        serde_json::from_slice(&std::fs::read(path.join("maker-note-request.json")).unwrap())
            .unwrap();
    let raw = std::fs::read(path.join("maker-note-evidence.json")).unwrap();
    let evidence: crate::notes::NoteEvidence = serde_json::from_slice(&raw).unwrap();
    let deployment: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path.join("sepolia.json")).unwrap()).unwrap();
    let now = deployment["ethereum_wallet"]["verified_at"]
        .as_u64()
        .unwrap();
    let verified = crate::notes::verify(&config, &request, &evidence, now).unwrap();
    assert_eq!(verified.execution_height, 12003717);
    assert_eq!(verified.pool.to_string().to_lowercase(), deployment["pool"]);
    for change in ["ancestry", "proof", "root", "signature"] {
        let mut changed: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        match change {
            "ancestry" => {
                changed["consensus"]["ancestry"][0]["stateRoot"] =
                    format!("0x{}", "01".repeat(32)).into()
            }
            "proof" => changed["pool"]["storageProof"][0]["value"] = "0x0".into(),
            "root" => {
                changed["consensus"]["execution"]["stateRoot"] =
                    format!("0x{}", "01".repeat(32)).into()
            }
            _ => {
                changed["consensus"]["finality"]["sync_aggregate"]["sync_committee_signature"] =
                    format!("0x{}", "00".repeat(96)).into()
            }
        }
        let changed = serde_json::from_value(changed).unwrap();
        assert!(
            crate::notes::verify(&config, &request, &changed, now).is_err(),
            "accepted {change}"
        );
    }
}
