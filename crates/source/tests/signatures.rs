#![allow(clippy::unwrap_used)]

use transparent::{address::Script, bundle::TxOut};
use zcash_primitives::transaction::Transaction;
use zcash_protocol::{consensus::BranchId, value::Zatoshis};
use zcash_script::script::Code;
use zutti_source::transaction::input_signature_hash;

#[test]
fn overwinter_and_sapling_hashes_match_published_zip_vectors() {
    for vector in zebra_test::zip0143::TEST_VECTORS.iter() {
        check(
            &vector.tx,
            &vector.script_code,
            vector.transparent_input,
            vector.hash_type,
            vector.amount,
            vector.consensus_branch_id,
            vector.sighash,
        );
    }
    for vector in zebra_test::zip0243::TEST_VECTORS.iter() {
        check(
            &vector.tx,
            &vector.script_code,
            vector.transparent_input,
            vector.hash_type,
            vector.amount,
            vector.consensus_branch_id,
            vector.sighash,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn check(
    bytes: &[u8],
    code: &[u8],
    input: Option<u32>,
    hash_type: u32,
    amount: i64,
    branch: u32,
    expected: [u8; 32],
) {
    let tx = Transaction::read(bytes, BranchId::try_from(branch).unwrap()).unwrap();
    let prevouts = tx.transparent_bundle().map_or_else(Vec::new, |b| {
        vec![
            TxOut::new(
                Zatoshis::from_u64(amount as u64).unwrap(),
                Script::default()
            );
            b.vin.len()
        ]
    });
    assert_eq!(
        input_signature_hash(
            &tx,
            &prevouts,
            input.map(|i| i as usize),
            &Code(code.to_vec()),
            hash_type as u8
        )
        .unwrap(),
        expected
    );
}

#[test]
fn zip244_hashes_bind_every_input_amount_and_script() {
    for vector in zebra_test::zip0244::TEST_VECTORS.iter() {
        let tx = Transaction::read(vector.tx.as_slice(), BranchId::Nu5).unwrap();
        let prevouts: Vec<_> = vector
            .amounts
            .iter()
            .zip(&vector.script_pubkeys)
            .map(|(value, script)| {
                TxOut::new(
                    Zatoshis::from_u64(*value as u64).unwrap(),
                    Script(Code(script.clone())),
                )
            })
            .collect();
        assert_eq!(
            input_signature_hash(&tx, &prevouts, None, &Code(vec![]), 1).unwrap(),
            vector.sighash_shielded
        );
        if let Some(input) = vector.transparent_input {
            for (kind, expected) in [
                (1, vector.sighash_all),
                (2, vector.sighash_none),
                (3, vector.sighash_single),
                (0x81, vector.sighash_all_anyone),
                (0x82, vector.sighash_none_anyone),
                (0x83, vector.sighash_single_anyone),
            ] {
                let result =
                    input_signature_hash(&tx, &prevouts, Some(input as usize), &Code(vec![]), kind);
                match expected {
                    Some(hash) => assert_eq!(result.unwrap(), hash),
                    None => assert!(result.is_err()),
                }
            }
        }
    }
}
