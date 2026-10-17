#![cfg(feature = "pool")]
#![allow(clippy::unwrap_used)]

use zcash_protocol::consensus::{MAIN_NETWORK, NetworkType};
use zutti_protocol::{
    Digest, Secret32,
    amount::{Amount, Asset, FeeBps},
    domain::Domain,
    encoding::Canonical,
    escrow::ReservationOpening,
    order::{Direction, OrderInput, OrderTerms},
    policy::PaymentWindow,
    pool::{OperationKind, PoolClaim, PoolTree, nullifier_digest, output_digest},
    private_note::PrivateNote,
};
use zutti_source::{block::ParsedBlock, bootstrap::GenesisBlock, state::SourceCandidate};
use zutti_wallet::{
    pool::{
        NoteInput, NoteOutput, PoolAction, PoolWitness, ReservationWitness, SettlementWitness,
        escrow::EscrowNote,
    },
    private_note::{NoteEncryption, NoteKeys},
};

fn secret(byte: u8) -> Secret32 {
    Secret32::from_bytes([byte; 32]).unwrap()
}
fn keys(byte: u8) -> NoteKeys {
    NoteKeys::from_secrets(secret(byte), secret(byte + 1), secret(byte + 2)).unwrap()
}
fn note(domain: Digest, owner: &NoteKeys, amount: u128, nonce: u8) -> PrivateNote {
    PrivateNote::new(
        domain,
        owner.recipient().unwrap(),
        Amount::from_units(Asset::Usdc, amount).unwrap(),
        secret(nonce),
        secret(nonce + 1),
    )
    .unwrap()
}
fn output(domain: Digest, owner: &NoteKeys, amount: u128, nonce: u8) -> NoteOutput {
    NoteOutput {
        note: note(domain, owner, amount, nonce),
        encryption: NoteEncryption::from_entropy(secret(nonce + 2), [nonce + 3; 24]),
    }
}
fn claim(
    kind: OperationKind,
    domain: Digest,
    root: Digest,
    value: u64,
    owner: [u8; 20],
    inputs: &[Digest],
    outputs: &[(Digest, Digest)],
) -> PoolClaim {
    PoolClaim {
        kind,
        domain,
        root,
        checkpoint: Digest::default(),
        order_tag: Digest::default(),
        order: Digest::default(),
        reservation: Digest::default(),
        value,
        owner,
        nullifiers: nullifier_digest(inputs).unwrap(),
        outputs: output_digest(outputs).unwrap(),
    }
}

#[test]
fn deposits_prove_the_exact_backed_value_and_decryptable_output() {
    let domain = Digest([3; 32]);
    let owner = keys(10);
    let output = output(domain, &owner, 100, 20);
    let claim = claim(
        OperationKind::Deposit,
        domain,
        Digest::default(),
        100,
        [4; 20],
        &[],
        &[output.binding().unwrap()],
    );
    let mut witness = PoolWitness {
        claim,
        inputs: vec![],
        outputs: vec![output],
        action: PoolAction::Deposit,
    };
    assert_eq!(witness.evaluate().unwrap(), witness.claim);
    let encoded = witness.to_bytes().unwrap();
    assert_eq!(
        PoolWitness::from_bytes(&encoded)
            .unwrap()
            .evaluate()
            .unwrap(),
        witness.claim
    );
    let recovered = owner
        .decrypt(&witness.outputs[0].encrypted().unwrap(), domain)
        .unwrap();
    assert_eq!(recovered.amount().units(), 100);
    witness.claim.value = 101;
    assert!(witness.evaluate().is_err());
    witness.claim.value = 100;
    witness.claim.outputs.0[0] ^= 1;
    assert!(witness.evaluate().is_err());
}

#[test]
fn transfers_and_withdrawals_require_membership_authority_and_conservation() {
    let domain = Digest([3; 32]);
    let owner = keys(10);
    let receiver = keys(30);
    let original = note(domain, &owner, 100, 20);
    let mut tree = PoolTree::new(domain).unwrap();
    tree.append(original.commitment().unwrap()).unwrap();
    let outputs = vec![
        output(domain, &receiver, 60, 40),
        output(domain, &owner, 40, 50),
    ];
    let claim = claim(
        OperationKind::Transfer,
        domain,
        tree.root(),
        0,
        [0; 20],
        &[owner.nullifier(&original).unwrap()],
        &outputs
            .iter()
            .map(|o| o.binding().unwrap())
            .collect::<Vec<_>>(),
    );
    let input = NoteInput::authorize(
        original,
        tree.prove(0).unwrap(),
        &owner,
        claim.journal_digest().unwrap(),
    )
    .unwrap();
    let mut witness = PoolWitness {
        claim,
        inputs: vec![input],
        outputs,
        action: PoolAction::Transfer,
    };
    witness.evaluate().unwrap();
    witness.outputs[0] = output(domain, &receiver, 61, 40);
    witness.claim.outputs = output_digest(
        &witness
            .outputs
            .iter()
            .map(|o| o.binding().unwrap())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    witness.inputs[0].authorization = owner
        .authorize(
            &witness.inputs[0].note,
            witness.claim.journal_digest().unwrap(),
        )
        .unwrap();
    assert!(witness.evaluate().is_err());
    witness.outputs = vec![output(domain, &owner, 75, 60)];
    witness.claim.kind = OperationKind::Withdraw;
    witness.claim.value = 25;
    witness.claim.owner = [7; 20];
    witness.claim.outputs = output_digest(&[witness.outputs[0].binding().unwrap()]).unwrap();
    witness.action = PoolAction::Withdraw;
    witness.inputs[0].authorization = owner
        .authorize(
            &witness.inputs[0].note,
            witness.claim.journal_digest().unwrap(),
        )
        .unwrap();
    witness.evaluate().unwrap();
    witness.claim.owner = [8; 20];
    assert!(witness.evaluate().is_err());
    witness.claim.owner = [7; 20];
    witness.inputs[0].path.position = 1;
    assert!(witness.evaluate().is_err());
}

fn source() -> (SourceCandidate, SourceCandidate) {
    let genesis =
        GenesisBlock::parse(zebra_test::vectors::MAINNET_BLOCKS[&0], NetworkType::Main).unwrap();
    let mut source = SourceCandidate::from_genesis(&genesis, Digest([5; 32]), 0).unwrap();
    for height in 1..=7 {
        source
            .append(
                &ParsedBlock::parse(zebra_test::vectors::MAINNET_BLOCKS[&height], &MAIN_NETWORK)
                    .unwrap(),
            )
            .unwrap();
    }
    let stable = source.clone();
    for height in 8..=10 {
        source
            .append(
                &ParsedBlock::parse(zebra_test::vectors::MAINNET_BLOCKS[&height], &MAIN_NETWORK)
                    .unwrap(),
            )
            .unwrap();
    }
    (source, stable)
}

#[test]
fn conditional_notes_release_only_the_earned_output_and_share_one_terminal_nullifier() {
    let (source, stable) = source();
    let genesis = stable.index_snapshot().unwrap().context.genesis();
    let domain = Domain::new(
        genesis,
        31337,
        [2; 20],
        [3; 20],
        Digest([6; 32]),
        Digest([5; 32]),
        Digest([7; 32]),
    )
    .unwrap();
    let id = domain.id().unwrap();
    let funder = keys(10);
    let receiver = keys(30);
    let checkpoint = source.checkpoint(3).unwrap();
    let anchor = source.prove_header(1).unwrap();
    let anchor_hash = ParsedBlock::parse(zebra_test::vectors::MAINNET_BLOCKS[&1], &MAIN_NETWORK)
        .unwrap()
        .hash();
    let included = ParsedBlock::parse(zebra_test::vectors::MAINNET_BLOCKS[&2], &MAIN_NETWORK)
        .unwrap()
        .txids()[0];
    for (paid, txid) in [(true, included), (false, Digest([90; 32]))] {
        let payout = output(id, &receiver, 199, 40);
        let refund = output(id, &funder, 199, 50);
        let terms = OrderTerms::new(OrderInput {
            domain: domain.clone(),
            direction: Direction::ZecToUsdc,
            offer: Digest([8; 32]),
            zec: Amount::parse(Asset::Zec, "5").unwrap(),
            usdc: Amount::from_units(Asset::Usdc, 199).unwrap(),
            receiver: [9; 43],
            payout: payout.fixed().unwrap(),
            refund: refund.fixed().unwrap(),
            gross_output: Amount::from_units(Asset::Usdc, 200).unwrap(),
            fee_bps: FeeBps::new(50).unwrap(),
            fee_caps: [
                Amount::from_units(Asset::Zec, 1000).unwrap(),
                Amount::from_units(Asset::Eth, 1000).unwrap(),
                Amount::from_units(Asset::Usdc, 1).unwrap(),
            ],
            window: PaymentWindow::new(0, 1, anchor_hash, 2).unwrap(),
            nonce: Digest([21; 32]),
        })
        .unwrap();
        let order = terms.commitment(&secret(60)).unwrap();
        let reservation = ReservationOpening::bind(id, order, txid, &secret(61)).unwrap();
        let escrow = EscrowNote::new(id, order, reservation, 199, secret(62), secret(63)).unwrap();
        let nullifier = escrow.nullifier().unwrap();
        let original = note(id, &funder, 250, 70);
        let mut tree = PoolTree::new(id).unwrap();
        tree.append(original.commitment().unwrap()).unwrap();
        let change = output(id, &funder, 51, 80);
        let reservation_witness = ReservationWitness {
            terms: terms.clone(),
            order_randomness: secret(60),
            reservation_randomness: secret(61),
            txid,
            escrow,
            nonce: [64; 24],
            checkpoint: checkpoint.clone(),
            anchor: anchor.clone(),
            payout,
            refund,
        };
        let mut public = claim(
            OperationKind::Reserve,
            id,
            tree.root(),
            0,
            [0; 20],
            &[funder.nullifier(&original).unwrap()],
            &[
                reservation_witness.binding().unwrap(),
                change.binding().unwrap(),
            ],
        );
        public.checkpoint = checkpoint.id().unwrap();
        public.order_tag = terms.use_tag().unwrap();
        public.order = order;
        public.reservation = reservation;
        let input = NoteInput::authorize(
            original,
            tree.prove(0).unwrap(),
            &funder,
            public.journal_digest().unwrap(),
        )
        .unwrap();
        let reserve = PoolWitness {
            claim: public,
            inputs: vec![input],
            outputs: vec![change],
            action: PoolAction::Reserve(Box::new(reservation_witness)),
        };
        reserve.evaluate().unwrap();
        let decoded = PoolWitness::from_bytes(&reserve.to_bytes().unwrap()).unwrap();
        decoded.evaluate().unwrap();
        let PoolAction::Reserve(reservation_witness) = reserve.action else {
            panic!("expected reservation");
        };
        let envelope = reservation_witness.escrow.encrypt([64; 24]).unwrap();
        let recovered = EscrowNote::decrypt(
            &envelope,
            id,
            reservation_witness.escrow.commitment().unwrap(),
            &secret(62),
        )
        .unwrap();
        assert_eq!(recovered.nullifier().unwrap(), nullifier);
        assert!(
            EscrowNote::decrypt(&envelope, id, recovered.commitment().unwrap(), &secret(63))
                .is_err()
        );
        tree.append(reservation_witness.escrow.commitment().unwrap())
            .unwrap();
        let chosen = if paid {
            output(id, &receiver, 199, 40)
        } else {
            output(id, &funder, 199, 50)
        };
        let mut public = claim(
            OperationKind::Settle,
            id,
            tree.root(),
            0,
            [0; 20],
            &[nullifier],
            &[chosen.binding().unwrap()],
        );
        public.checkpoint = checkpoint.id().unwrap();
        let settlement = SettlementWitness {
            terms,
            order_randomness: secret(60),
            reservation_randomness: secret(61),
            txid,
            escrow: recovered,
            path: tree.prove(1).unwrap(),
            checkpoint: checkpoint.clone(),
            anchor: anchor.clone(),
            payment: stable.prove(txid).unwrap(),
        };
        let mut witness = PoolWitness {
            claim: public,
            inputs: vec![],
            outputs: vec![chosen],
            action: PoolAction::Settle(Box::new(settlement)),
        };
        witness.evaluate().unwrap();
        assert_eq!(
            PoolWitness::from_bytes(&witness.to_bytes().unwrap())
                .unwrap()
                .evaluate()
                .unwrap(),
            witness.claim
        );
        witness.outputs[0] = if paid {
            output(id, &funder, 199, 50)
        } else {
            output(id, &receiver, 199, 40)
        };
        witness.claim.outputs = output_digest(&[witness.outputs[0].binding().unwrap()]).unwrap();
        assert!(witness.evaluate().is_err());
    }
}
