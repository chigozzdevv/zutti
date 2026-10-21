use zcash_protocol::consensus::{MAIN_NETWORK, NetworkType};
use zutti_protocol::{
    Digest, Secret32,
    amount::{Amount, Asset, FeeBps},
    domain::Domain,
    escrow::ReservationOpening,
    order::{Direction, OrderInput, OrderTerms},
    policy::PaymentWindow,
    pool::{OperationKind, PoolClaim, PoolTree, nullifier_digest, output_digest},
    private_note::PrivateNote,
};
use zutti_source::{block::ParsedBlock, bootstrap::GenesisBlock, state::SourceCandidate};
use zutti_wallet::{
    pool::{
        NoteInput, NoteOutput, PoolAction, PoolWitness, ReservationWitness, escrow::EscrowNote,
    },
    private_note::{NoteEncryption, NoteKeys},
    swap::{SettlementEvidence, SwapRecord},
};

pub fn secret(byte: u8) -> Secret32 {
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
fn output(domain: Digest, owner: &NoteKeys, nonce: u8) -> NoteOutput {
    NoteOutput {
        note: note(domain, owner, 199, nonce),
        encryption: NoteEncryption::from_entropy(secret(nonce + 2), [nonce + 3; 24]),
    }
}

pub fn scenario(
    paid: bool,
    direction: Direction,
    expiry: u32,
) -> (PoolWitness, SettlementEvidence, NoteKeys, NoteKeys) {
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
    let domain = Domain::new(
        stable.index_snapshot().unwrap().context.genesis(),
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
    let txid = if paid {
        ParsedBlock::parse(zebra_test::vectors::MAINNET_BLOCKS[&2], &MAIN_NETWORK)
            .unwrap()
            .txids()[0]
    } else {
        Digest([90; 32])
    };
    let payout = output(id, &receiver, 40);
    let refund = output(id, &funder, 50);
    let zec = Amount::parse(Asset::Zec, "5").unwrap();
    let terms = OrderTerms::new(OrderInput {
        domain,
        direction,
        offer: Digest([8; 32]),
        zec,
        usdc: Amount::from_units(Asset::Usdc, 199).unwrap(),
        receiver: [9; 43],
        payout: payout.fixed().unwrap(),
        refund: refund.fixed().unwrap(),
        gross_output: match direction {
            Direction::ZecToUsdc => Amount::from_units(Asset::Usdc, 200).unwrap(),
            Direction::UsdcToZec => zec,
        },
        fee_bps: FeeBps::new(if direction == Direction::ZecToUsdc {
            50
        } else {
            0
        })
        .unwrap(),
        fee_caps: [
            Amount::from_units(Asset::Zec, 1000).unwrap(),
            Amount::from_units(Asset::Eth, 1000).unwrap(),
            Amount::from_units(Asset::Usdc, 1).unwrap(),
        ],
        window: PaymentWindow::new(
            0,
            1,
            ParsedBlock::parse(zebra_test::vectors::MAINNET_BLOCKS[&1], &MAIN_NETWORK)
                .unwrap()
                .hash(),
            expiry,
        )
        .unwrap(),
        nonce: Digest([21; 32]),
    })
    .unwrap();
    let order = terms.commitment(&secret(60)).unwrap();
    let reservation = ReservationOpening::bind(id, order, txid, &secret(61)).unwrap();
    let escrow = EscrowNote::new(id, order, reservation, 199, secret(62), secret(63)).unwrap();
    let original = note(id, &funder, 199, 70);
    let mut tree = PoolTree::new(id).unwrap();
    tree.append(original.commitment().unwrap()).unwrap();
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
    let funding = PoolClaim {
        kind: OperationKind::Reserve,
        domain: id,
        root: tree.root(),
        checkpoint: checkpoint.id().unwrap(),
        order_tag: terms.use_tag().unwrap(),
        order,
        reservation,
        value: 0,
        owner: [0; 20],
        nullifiers: nullifier_digest(&[funder.nullifier(&original).unwrap()]).unwrap(),
        outputs: output_digest(&[reservation_witness.binding().unwrap()]).unwrap(),
    };
    let funding_path = tree.prove(0).unwrap();
    tree.append(reservation_witness.escrow.commitment().unwrap())
        .unwrap();
    let input = NoteInput::authorize(
        original,
        funding_path,
        &funder,
        funding.journal_digest().unwrap(),
    )
    .unwrap();
    let reserve = PoolWitness {
        claim: funding,
        inputs: vec![input],
        outputs: vec![],
        action: PoolAction::Reserve(Box::new(reservation_witness)),
    };
    reserve.evaluate().unwrap();
    let evidence = SettlementEvidence {
        swap: SwapRecord::from_reservation(&reserve)
            .unwrap()
            .id()
            .unwrap(),
        root: tree.root(),
        path: tree.prove(1).unwrap(),
        checkpoint,
        anchor,
        payment: stable.prove(txid).unwrap(),
    };
    (reserve, evidence, receiver, funder)
}
