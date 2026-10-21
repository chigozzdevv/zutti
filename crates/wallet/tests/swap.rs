#![cfg(feature = "native")]
#![allow(clippy::unwrap_used)]

#[path = "../../../tests/support/swap.rs"]
mod support;

use support::{scenario, secret};
use zutti_protocol::{Digest, encoding::Canonical, order::Direction};
use zutti_wallet::{
    pool::{PoolAction, PoolWitness},
    swap::{MAX_SWAP_RECORD, SettlementAction, SettlementEvidence, SwapRecord},
};

#[test]
fn both_directions_restore_the_fixed_output_without_funding_spend_secrets() {
    for direction in [Direction::ZecToUsdc, Direction::UsdcToZec] {
        for paid in [true, false] {
            let (reserve, evidence, receiver, funder) = scenario(paid, direction, 2);
            let record = SwapRecord::from_reservation(&reserve).unwrap();
            let bytes = record.to_bytes().unwrap();
            let input_secret = reserve.inputs[0].nullifier_key.expose();
            assert!(!bytes.windows(32).any(|window| window == input_secret));
            let restored = SwapRecord::from_bytes(&bytes).unwrap();
            assert_eq!(restored.id().unwrap(), record.id().unwrap());
            let action = if paid {
                SettlementAction::Claim
            } else {
                SettlementAction::Refund
            };
            let witness = restored.settlement(action, &evidence).unwrap();
            witness.evaluate().unwrap();
            let owner = if paid { &receiver } else { &funder };
            let other = if paid { &funder } else { &receiver };
            let encrypted = witness.outputs[0].encrypted().unwrap();
            let recovered = owner.decrypt(&encrypted, record.domain()).unwrap();
            assert_eq!(recovered.amount().units(), 199);
            assert!(other.decrypt(&encrypted, record.domain()).is_err());
            let opposite = if paid {
                SettlementAction::Refund
            } else {
                SettlementAction::Claim
            };
            assert!(restored.settlement(opposite, &evidence).is_err());
            assert_eq!(
                PoolWitness::from_bytes(&witness.to_bytes().unwrap())
                    .unwrap()
                    .evaluate()
                    .unwrap(),
                witness.claim
            );
        }
    }
}

#[test]
fn archive_restore_checks_identity_key_domain_and_refuses_overwrite() {
    let (reserve, _, _, _) = scenario(true, Direction::ZecToUsdc, 2);
    let record = SwapRecord::from_reservation(&reserve).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("swap.backup");
    record.save_new(&path, &secret(111)).unwrap();
    assert!(record.save_new(&path, &secret(111)).is_err());
    let read = |key, domain, id| SwapRecord::read(&path, &key, domain, id);
    assert!(read(secret(111), record.domain(), record.id().unwrap()).is_ok());
    assert!(read(secret(112), record.domain(), record.id().unwrap()).is_err());
    assert!(read(secret(111), Digest([113; 32]), record.id().unwrap()).is_err());
    assert!(read(secret(111), record.domain(), Digest([114; 32])).is_err());
    assert!(
        !std::fs::read(path)
            .unwrap()
            .windows(8)
            .any(|w| w == b"ZUTTISWP")
    );
}

#[test]
fn expired_quote_does_not_revoke_a_paid_claim_and_missing_payment_before_expiry_waits() {
    let (reserve, evidence, _, _) = scenario(true, Direction::ZecToUsdc, 2);
    assert!(evidence.checkpoint.stable.height > 2);
    let record = SwapRecord::from_reservation(&reserve).unwrap();
    assert!(
        record
            .settlement(SettlementAction::Claim, &evidence)
            .is_ok()
    );
    assert!(
        record
            .settlement(SettlementAction::Refund, &evidence)
            .is_err()
    );
    let (reserve, evidence, _, _) = scenario(false, Direction::ZecToUsdc, 8);
    let record = SwapRecord::from_reservation(&reserve).unwrap();
    for action in [SettlementAction::Claim, SettlementAction::Refund] {
        assert!(record.settlement(action, &evidence).is_err());
    }
}

#[test]
fn settlement_rejects_changed_swap_pool_membership_ancestry_and_payment_path() {
    let (reserve, evidence, _, _) = scenario(true, Direction::ZecToUsdc, 2);
    let record = SwapRecord::from_reservation(&reserve).unwrap();
    for changed in 0..6 {
        let mut evidence = evidence.clone();
        match changed {
            0 => evidence.swap.0[0] ^= 1,
            1 => evidence.root.0[0] ^= 1,
            2 => evidence.path.position ^= 1,
            3 => evidence.checkpoint.archive.0[0] ^= 1,
            4 => evidence.path.siblings[0].0[0] ^= 1,
            _ => {
                let (_, absent, _, _) = scenario(false, Direction::ZecToUsdc, 2);
                evidence.payment = absent.payment;
            }
        }
        assert!(
            record
                .settlement(SettlementAction::Claim, &evidence)
                .is_err()
        );
    }
}

#[test]
fn only_valid_reservations_create_records_and_decoding_is_bounded_and_strict() {
    let (mut reserve, evidence, _, _) = scenario(true, Direction::ZecToUsdc, 2);
    let record = SwapRecord::from_reservation(&reserve).unwrap();
    let mut bytes = record.to_bytes().unwrap();
    bytes.push(0);
    assert!(SwapRecord::from_bytes(&bytes).is_err());
    assert!(SwapRecord::from_bytes(&vec![0; MAX_SWAP_RECORD + 1]).is_err());
    assert!(SwapRecord::from_bytes(&bytes[..bytes.len() / 2]).is_err());
    let mut bytes = record.to_bytes().unwrap();
    bytes[10 + 11] ^= 1;
    assert!(SwapRecord::from_bytes(&bytes).is_err());
    reserve.claim.outputs.0[0] ^= 1;
    assert!(SwapRecord::from_reservation(&reserve).is_err());
    reserve.action = PoolAction::Transfer;
    assert!(SwapRecord::from_reservation(&reserve).is_err());
    let mut bytes = evidence.to_bytes().unwrap();
    bytes.push(0);
    assert!(SettlementEvidence::from_bytes(&bytes).is_err());
    assert_eq!(format!("{record:?}"), "SwapRecord([REDACTED])");
    assert_eq!(format!("{evidence:?}"), "SettlementEvidence([REDACTED])");
}
