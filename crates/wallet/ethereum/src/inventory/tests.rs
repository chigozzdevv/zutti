#![allow(clippy::unwrap_used)]
use super::*;
use alloy::{
    consensus::TrieAccount,
    primitives::{Bytes, keccak256},
    rlp,
    rpc::types::EIP1186StorageProof,
};
use alloy_trie::{HashBuilder, Nibbles, proof::ProofRetainer};

fn trie(leaves: Vec<(Vec<u8>, Vec<u8>)>, keys: &[Vec<u8>]) -> (B256, Vec<Vec<Bytes>>) {
    let targets: Vec<_> = keys.iter().map(|k| Nibbles::unpack(keccak256(k))).collect();
    let mut builder =
        HashBuilder::default().with_proof_retainer(ProofRetainer::new(targets.clone()));
    let mut leaves: Vec<_> = leaves
        .into_iter()
        .map(|(k, v)| (Nibbles::unpack(keccak256(k)), v))
        .collect();
    leaves.sort_unstable_by_key(|v| v.0);
    for (key, value) in leaves {
        builder.add_leaf(key, &value);
    }
    let root = builder.root();
    let nodes = builder.take_proof_nodes();
    (
        root,
        targets
            .iter()
            .map(|target| {
                nodes
                    .matching_nodes_sorted(target)
                    .into_iter()
                    .map(|(_, v)| v)
                    .collect()
            })
            .collect(),
    )
}

fn storage(
    address: Address,
    code_hash: B256,
    values: &[(B256, U256)],
) -> EIP1186AccountProofResponse {
    let keys: Vec<_> = values.iter().map(|v| v.0.to_vec()).collect();
    let (root, proofs) = trie(
        values
            .iter()
            .filter(|v| !v.1.is_zero())
            .map(|v| (v.0.to_vec(), rlp::encode(v.1)))
            .collect(),
        &keys,
    );
    EIP1186AccountProofResponse {
        address,
        nonce: 1,
        balance: U256::ZERO,
        storage_hash: root,
        code_hash,
        account_proof: vec![],
        storage_proof: values
            .iter()
            .zip(proofs)
            .map(|((key, value), proof)| EIP1186StorageProof {
                key: (*key).into(),
                value: *value,
                proof,
            })
            .collect(),
    }
}

fn fixture(
    change: u8,
) -> (
    InventoryRequest,
    EIP1186AccountProofResponse,
    EIP1186AccountProofResponse,
    B256,
) {
    let request = InventoryRequest {
        chain_id: 11155111,
        pool: Address::repeat_byte(1),
        pool_code_hash: B256::repeat_byte(2),
        source: Address::repeat_byte(3),
        source_code_hash: B256::repeat_byte(4),
        root: B256::repeat_byte(5),
        checkpoint: B256::repeat_byte(6),
        order_tag: B256::repeat_byte(7),
        nullifiers: vec![B256::repeat_byte(8)],
        expiry: 106,
        confirmations: 2,
        minimum_remaining: 3,
        maximum_source_age: 90,
        future_drift: 30,
    };
    let one = U256::from(1);
    let zero = U256::ZERO;
    let mut pool = storage(
        request.pool,
        request.pool_code_hash,
        &[
            (
                mapping_key(request.root, 68),
                if change == 1 { zero } else { one },
            ),
            (
                mapping_key(request.order_tag, 71),
                if change == 2 { one } else { zero },
            ),
            (
                mapping_key(request.nullifiers[0], 69),
                if change == 3 { one } else { zero },
            ),
        ],
    );
    let mut source = storage(
        request.source,
        request.source_code_hash,
        &[
            (
                B256::from(U256::from(2).to_be_bytes::<32>()),
                U256::from(if change == 6 { 102 } else { 100 }) | (U256::from(200) << 32),
            ),
            (
                B256::from(U256::from(4).to_be_bytes::<32>()),
                if change == 4 {
                    one << 64
                } else {
                    U256::from(123)
                },
            ),
            (
                mapping_key(request.checkpoint, 19),
                if change == 5 { zero } else { one },
            ),
        ],
    );
    let accounts: Vec<_> = [&pool, &source]
        .iter()
        .map(|proof| {
            (
                proof.address.to_vec(),
                rlp::encode(TrieAccount {
                    nonce: proof.nonce,
                    balance: proof.balance,
                    storage_root: proof.storage_hash,
                    code_hash: proof.code_hash,
                }),
            )
        })
        .collect();
    let (root, mut proofs) = trie(accounts, &[pool.address.to_vec(), source.address.to_vec()]);
    source.account_proof = proofs.pop().unwrap();
    pool.account_proof = proofs.pop().unwrap();
    (request, pool, source, root)
}

#[test]
fn verifies_inventory_and_source_at_one_authenticated_state_root() {
    let (request, pool, source, root) = fixture(0);
    verify_inventory_storage(&request, &pool, &source, root, 220).unwrap();
    let mut altered = root;
    altered[0] ^= 1;
    assert!(verify_inventory_storage(&request, &pool, &source, altered, 220).is_err());
    let (_, _, alternate, _) = fixture(4);
    assert!(verify_inventory_storage(&request, &pool, &alternate, root, 220).is_err());
}

#[test]
fn genuinely_proved_ineligible_states_cannot_authorize_funding() {
    for change in 1..=6 {
        let (request, pool, source, root) = fixture(change);
        assert!(
            verify_inventory_storage(&request, &pool, &source, root, 220).is_err(),
            "state {change}"
        );
    }
}

#[test]
fn source_freshness_and_future_drift_enforce_exact_boundaries() {
    let (request, pool, source, root) = fixture(0);
    for now in [170, 290] {
        verify_inventory_storage(&request, &pool, &source, root, now).unwrap();
    }
    for now in [169, 291] {
        assert!(verify_inventory_storage(&request, &pool, &source, root, now).is_err());
    }
}

#[test]
fn missing_duplicate_and_modified_proof_slots_fail() {
    let (request, pool, source, root) = fixture(0);
    for field in 0..4 {
        let mut changed = pool.clone();
        match field {
            0 => {
                changed.storage_proof.pop();
            }
            1 => changed.storage_proof[1] = changed.storage_proof[0].clone(),
            2 => changed.storage_proof[0].value = U256::ZERO,
            _ => changed.code_hash[0] ^= 1,
        }
        assert!(verify_inventory_storage(&request, &changed, &source, root, 220).is_err());
    }
}

#[test]
fn request_limits_and_duplicate_nullifiers_are_checked_before_proofs() {
    for field in 0..6 {
        let (mut request, _, _, _) = fixture(0);
        match field {
            0 => request.nullifiers.push(request.nullifiers[0]),
            1 => request.nullifiers[0] = B256::ZERO,
            2 => request.confirmations = 0,
            3 => request.minimum_remaining = 129,
            4 => request.source = request.pool,
            _ => request.future_drift = 7201,
        }
        assert!(check_request(&request).is_err());
    }
}
