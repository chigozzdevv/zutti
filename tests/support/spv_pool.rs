#![allow(dead_code, clippy::unwrap_used)]
use super::{payment, support};
use zcash_protocol::consensus::NetworkType;
use zutti_protocol::{
    Digest,
    amount::{Amount, Asset},
    domain::Domain,
    encoding::{Canonical, Writer},
    pool::{OperationKind, PoolClaim, PoolTree, nullifier_digest, output_digest},
    private_note::PrivateNote,
};
use zutti_source::spv::{HeaderClaim, Policy, WindowClaim};
use zutti_wallet::{
    binding::{PaymentBindingWitness, payment_profile},
    pool::{NoteInput, NoteOutput, escrow::EscrowNote},
    preparation::PreparationWitness,
    private_note::{NoteEncryption, NoteKeys},
    spv_pool::{SpvAction, SpvPoolClaim, SpvPoolWitness, SpvReservation},
};

pub fn keys(byte: u8) -> NoteKeys {
    NoteKeys::from_secrets(
        support::secret(byte),
        support::secret(byte + 1),
        support::secret(byte + 2),
    )
    .unwrap()
}
pub fn output(domain: Digest, owner: &NoteKeys, amount: u128, nonce: u8) -> NoteOutput {
    NoteOutput {
        note: PrivateNote::new(
            domain,
            owner.recipient().unwrap(),
            Amount::from_units(Asset::Usdc, amount).unwrap(),
            support::secret(nonce),
            support::secret(nonce + 1),
        )
        .unwrap(),
        encryption: NoteEncryption::from_entropy(support::secret(nonce + 2), [nonce + 3; 24]),
    }
}
pub fn unverified_anchor(policy: Policy) -> HeaderClaim {
    let mut writer = Writer::default();
    writer.fixed(b"ZUTTISPH");
    writer.u16(2);
    policy.encode(&mut writer).unwrap();
    writer.u8(2);
    writer.u8(113);
    for height in 4_464_988..=4_465_100 {
        writer.u32(height);
        writer.fixed(&support::digest(13).0);
        writer.u32(1_800_000_000 + height);
        writer.u32(0x2007_ffff);
    }
    writer.fixed(&[1; 32]);
    writer.u32(1_804_465_100);
    HeaderClaim::from_bytes(&writer.finish()).unwrap()
}
pub fn reservation(binding_program: Digest, source_program: Digest) -> SpvPoolWitness {
    reservation_for_program(binding_program, source_program, support::digest(60))
}
pub fn reservation_for_program(
    binding_program: Digest,
    source_program: Digest,
    pool_program: Digest,
) -> SpvPoolWitness {
    let policy = Policy::new(source_program, NetworkType::Test, 2, 128, 4_465_200).unwrap();
    let domain = Domain::new(
        zutti_protocol::network::ZCASH_TESTNET_GENESIS,
        11155111,
        [2; 20],
        [3; 20],
        payment_profile(),
        policy.id().unwrap(),
        support::digest(6),
    )
    .unwrap();
    let payer = keys(10);
    let maker = keys(20);
    let payout = output(domain.id().unwrap(), &payer, 199_000_000, 40);
    let refund = output(domain.id().unwrap(), &maker, 199_000_000, 50);
    let payment = payment::fixture_custom(false, "0.001", 4_465_101, |input| {
        input.domain = domain.clone();
        input.payout = payout.fixed().unwrap();
        input.refund = refund.fixed().unwrap();
    });
    let inspected = zutti_wallet::prepare::inspect_ironwood(
        &payment.pczt.clone().serialize().unwrap(),
        &payment.terms,
        &support::secret(21),
    )
    .unwrap();
    let binding = PaymentBindingWitness::new(
        policy,
        PreparationWitness::new(
            payment.terms.clone(),
            support::secret(21),
            support::secret(32),
            payment.pczt.serialize().unwrap(),
        )
        .unwrap(),
    )
    .unwrap()
    .evaluate()
    .unwrap();
    let escrow = EscrowNote::new(
        domain.id().unwrap(),
        binding.order(),
        binding.reservation(),
        199_000_000,
        support::secret(70),
        support::secret(71),
    )
    .unwrap();
    let reservation = SpvReservation {
        terms: payment.terms,
        order_randomness: support::secret(21),
        reservation_randomness: support::secret(32),
        txid: inspected.txid(),
        escrow,
        nonce: [73; 24],
        anchor: unverified_anchor(policy),
        binding,
        payout,
        refund,
    };
    let original = output(domain.id().unwrap(), &maker, 300_000_000, 80).note;
    let mut tree = PoolTree::new(domain.id().unwrap()).unwrap();
    tree.append(original.commitment().unwrap()).unwrap();
    let change = output(domain.id().unwrap(), &maker, 101_000_000, 90);
    let claim = SpvPoolClaim {
        policy: policy.id().unwrap(),
        binding_program,
        pool_program,
        expiry: reservation.terms.window().expiry(),
        pool: PoolClaim {
            kind: OperationKind::Reserve,
            domain: domain.id().unwrap(),
            root: tree.root(),
            checkpoint: reservation.anchor.id().unwrap(),
            order_tag: reservation.terms.use_tag().unwrap(),
            order: reservation.binding.order(),
            reservation: reservation.binding.reservation(),
            value: 0,
            owner: [0; 20],
            nullifiers: nullifier_digest(&[maker.nullifier(&original).unwrap()]).unwrap(),
            outputs: output_digest(&[reservation.binding().unwrap(), change.binding().unwrap()])
                .unwrap(),
        },
    };
    let input = NoteInput::authorize(
        original,
        tree.prove(0).unwrap(),
        &maker,
        claim.journal_digest().unwrap(),
    )
    .unwrap();
    SpvPoolWitness {
        policy,
        claim,
        inputs: vec![input],
        outputs: vec![change],
        action: SpvAction::Reserve(Box::new(reservation)),
    }
}
pub fn terminal(funding: &SpvPoolWitness, paid: bool) -> SpvPoolWitness {
    let SpvAction::Reserve(original) = &funding.action else {
        panic!("funding fixture")
    };
    let reservation = SpvReservation::from_bytes(&original.to_bytes().unwrap()).unwrap();
    let output = NoteOutput::from_bytes(
        &if paid {
            &reservation.payout
        } else {
            &reservation.refund
        }
        .to_bytes()
        .unwrap(),
    )
    .unwrap();
    let mut tree = PoolTree::new(funding.claim.pool.domain).unwrap();
    tree.append(funding.inputs[0].note.commitment().unwrap())
        .unwrap();
    tree.append(reservation.escrow.commitment().unwrap())
        .unwrap();
    tree.append(funding.outputs[0].note.commitment().unwrap())
        .unwrap();
    let window = WindowClaim {
        policy: funding.claim.policy,
        prefix: reservation.anchor.id().unwrap(),
        checkpoint: support::digest(100),
        reservation: reservation.binding.source_reservation(),
        height: 4_465_125,
        tip: support::digest(101),
        work: [2; 32],
        output: output.note.commitment().unwrap(),
    };
    let claim = SpvPoolClaim {
        policy: funding.claim.policy,
        binding_program: funding.claim.binding_program,
        pool_program: funding.claim.pool_program,
        expiry: 0,
        pool: PoolClaim {
            kind: OperationKind::Settle,
            domain: funding.claim.pool.domain,
            root: tree.root(),
            checkpoint: window.checkpoint,
            order_tag: Digest::default(),
            order: Digest::default(),
            reservation: Digest::default(),
            value: 0,
            owner: [0; 20],
            nullifiers: nullifier_digest(&[reservation.escrow.nullifier().unwrap()]).unwrap(),
            outputs: output_digest(&[output.binding().unwrap()]).unwrap(),
        },
    };
    SpvPoolWitness {
        policy: funding.policy,
        claim,
        inputs: vec![],
        outputs: vec![output],
        action: SpvAction::Settle {
            funded_outputs: vec![
                reservation.binding().unwrap(),
                funding.outputs[0].binding().unwrap(),
            ],
            funding: Box::new(funding.claim.clone()),
            reservation: Box::new(reservation),
            path: Box::new(tree.prove(1).unwrap()),
            window: Box::new(window),
        },
    }
}
