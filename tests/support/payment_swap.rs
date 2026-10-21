#![allow(dead_code, clippy::unwrap_used)]
use super::{payment_fixture as payment, support};
use payment::Fixture;
use zutti_protocol::{
    Digest,
    amount::{Amount, Asset},
    commitment::hash,
    encoding::Canonical,
    escrow::ReservationOpening,
    pool::{OperationKind, PoolClaim, PoolTree, nullifier_digest, output_digest},
    private_note::PrivateNote,
};
use zutti_source::{
    IndexContext, IndexSnapshot,
    authenticated::AuthenticatedMap,
    checkpoint::{HeaderPath, HeaderRecord, SourceCheckpoint},
};
use zutti_wallet::{
    pool::{
        NoteInput, NoteOutput, PoolAction, PoolWitness, ReservationWitness, escrow::EscrowNote,
    },
    prepare::inspect_ironwood,
    private_note::{NoteEncryption, NoteKeys},
    swap::SwapRecord,
};

pub fn fixture() -> (Fixture, SwapRecord) {
    fixture_with_policy(Digest([5; 32]))
}
pub fn fixture_with_policy(policy: Digest) -> (Fixture, SwapRecord) {
    let base = payment::fixture(false);
    let original = base.terms.domain();
    let deployment = zutti_protocol::domain::Domain::new(
        original.source_genesis(),
        original.destination_chain(),
        original.pool(),
        [3; 20],
        original.payment_profile(),
        policy,
        original.proof_profile(),
    )
    .unwrap();
    let domain = deployment.id().unwrap();
    let keys = |n| {
        NoteKeys::from_secrets(
            support::secret(n),
            support::secret(n + 1),
            support::secret(n + 2),
        )
        .unwrap()
    };
    let funder = keys(100);
    let receiver = keys(110);
    let output = |owner: &NoteKeys, n| NoteOutput {
        note: PrivateNote::new(
            domain,
            owner.recipient().unwrap(),
            Amount::parse(Asset::Usdc, "199").unwrap(),
            support::secret(n),
            support::secret(n + 1),
        )
        .unwrap(),
        encryption: NoteEncryption::from_entropy(support::secret(n + 2), [n + 3; 24]),
    };
    let payout = output(&receiver, 120);
    let refund = output(&funder, 130);
    let fixture = payment::fixture_custom(false, "0.001", 4_465_101, |input| {
        input.domain = deployment;
        input.payout = payout.fixed().unwrap();
        input.refund = refund.fixed().unwrap();
    });
    let terms = &fixture.terms;
    let window = terms.window();
    let context = IndexContext::new(
        terms.domain().source_genesis(),
        terms.domain().source_policy(),
        window.index_start(),
    )
    .unwrap();
    let stable = HeaderRecord {
        height: window.anchor_height(),
        hash: window.anchor_hash(),
        work: [1; 32],
        index: IndexSnapshot {
            context,
            height: window.anchor_height(),
            root: Digest([20; 32]),
        },
    };
    let mut archive = AuthenticatedMap::new(hash(
        b"source-header-archive",
        &[&context.genesis().0, &context.rules().0],
    ));
    let key = hash(b"source-header-height", &[&stable.height.to_le_bytes()]);
    archive.set(key, Some(stable.to_bytes().unwrap())).unwrap();
    let checkpoint = SourceCheckpoint {
        stable,
        archive: archive.root(),
    };
    let anchor = HeaderPath {
        access: archive.prove(key).unwrap(),
    };
    let txid = inspect_ironwood(
        &fixture.pczt.clone().serialize().unwrap(),
        terms,
        &fixture.randomness,
    )
    .unwrap()
    .txid();
    let order = terms.commitment(&fixture.randomness).unwrap();
    let reservation = ReservationOpening::bind(domain, order, txid, &support::secret(140)).unwrap();
    let escrow = EscrowNote::new(
        domain,
        order,
        reservation,
        terms.usdc().units(),
        support::secret(141),
        support::secret(142),
    )
    .unwrap();
    let reservation = ReservationWitness {
        terms: terms.clone(),
        order_randomness: support::secret(21),
        reservation_randomness: support::secret(140),
        txid,
        escrow,
        nonce: [143; 24],
        checkpoint: checkpoint.clone(),
        anchor,
        payout,
        refund,
    };
    let note = output(&funder, 150).note;
    let mut tree = PoolTree::new(domain).unwrap();
    tree.append(note.commitment().unwrap()).unwrap();
    let claim = PoolClaim {
        kind: OperationKind::Reserve,
        domain,
        root: tree.root(),
        checkpoint: checkpoint.id().unwrap(),
        order_tag: terms.use_tag().unwrap(),
        order,
        reservation: reservation.escrow.reservation,
        value: 0,
        owner: [0; 20],
        nullifiers: nullifier_digest(&[funder.nullifier(&note).unwrap()]).unwrap(),
        outputs: output_digest(&[reservation.binding().unwrap()]).unwrap(),
    };
    let input = NoteInput::authorize(
        note,
        tree.prove(0).unwrap(),
        &funder,
        claim.journal_digest().unwrap(),
    )
    .unwrap();
    let witness = PoolWitness {
        claim,
        inputs: vec![input],
        outputs: vec![],
        action: PoolAction::Reserve(Box::new(reservation)),
    };
    (fixture, SwapRecord::from_reservation(&witness).unwrap())
}
