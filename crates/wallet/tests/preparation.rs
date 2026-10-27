#![allow(clippy::unwrap_used)]
#[path = "../../../tests/support/payment.rs"]
mod payment;
#[path = "../../../tests/support/mod.rs"]
mod support;
use orchard::keys::SpendAuthorizingKey;
use payment::{fixture, fixture_with_cap};
use pczt::{Pczt, roles::signer::Signer};
use rand::{SeedableRng, rngs::StdRng};
use zcash_primitives::transaction::txid::{TxIdDigester, to_txid};
use zutti_protocol::{Digest, order::OrderTerms};
use zutti_wallet::{Error, prepare::inspect_ironwood};

#[test]
fn prepared_source_fee_cannot_exceed_the_agreed_cap() {
    let fixture = fixture_with_cap(false, "0.00000001");
    let bytes = fixture.pczt.serialize().unwrap();
    assert!(matches!(
        inspect_ironwood(&bytes, &fixture.terms, &fixture.randomness),
        Err(Error::Payment)
    ));
}

#[test]
fn payment_branch_matches_the_network_and_entire_inclusion_window() {
    for height in [4_460_000, 4_465_026, 4_465_101] {
        let fixture = payment::fixture_at(false, "0.001", height);
        assert!(
            inspect_ironwood(
                &fixture.pczt.serialize().unwrap(),
                &fixture.terms,
                &fixture.randomness
            )
            .is_ok()
        );
    }
    let crossing = payment::fixture_at(false, "0.001", 4_465_025);
    assert!(matches!(
        inspect_ironwood(
            &crossing.pczt.serialize().unwrap(),
            &crossing.terms,
            &crossing.randomness
        ),
        Err(Error::Payment)
    ));
    let fixture = fixture(false);
    let mut input = support::input();
    input.receiver = *fixture.terms.receiver();
    input.window = fixture.terms.window();
    let unknown_network = OrderTerms::new(input).unwrap();
    assert!(matches!(
        inspect_ironwood(
            &fixture.pczt.serialize().unwrap(),
            &unknown_network,
            &fixture.randomness
        ),
        Err(Error::Payment)
    ));
}
fn effect_id(pczt: Pczt) -> Digest {
    let effects = pczt.into_effects().unwrap();
    Digest(
        *to_txid(
            effects.version(),
            effects.consensus_branch_id(),
            &effects.digest(TxIdDigester),
        )
        .as_ref(),
    )
}
#[test]
fn ironwood_preparation_preserves_transaction_id_after_authorization() {
    let fixture = fixture(false);
    let bytes = fixture.pczt.clone().serialize().unwrap();
    let inspected = inspect_ironwood(&bytes, &fixture.terms, &fixture.randomness).unwrap();
    assert_eq!(inspected.txid(), effect_id(fixture.pczt.clone()));
    let mut signer = Signer::new(fixture.pczt).unwrap();
    signer
        .sign_ironwood(
            StdRng::from_seed([44; 32]),
            fixture.spend,
            &SpendAuthorizingKey::from(&fixture.key),
        )
        .unwrap();
    let signed = signer.finish();
    assert_eq!(inspected.txid(), effect_id(signed.clone()));
    assert!(matches!(
        inspect_ironwood(
            &signed.serialize().unwrap(),
            &fixture.terms,
            &fixture.randomness
        ),
        Err(Error::AlreadyAuthorized)
    ));
    assert!(!format!("{inspected:?}").contains(&inspected.txid().to_string()));
}
#[test]
fn wrong_encrypted_memo_and_modified_terms_are_rejected() {
    let wrong = fixture(true);
    assert!(
        inspect_ironwood(
            &wrong.pczt.serialize().unwrap(),
            &wrong.terms,
            &wrong.randomness
        )
        .is_err()
    );
    let fixture = fixture(false);
    let bytes = fixture.pczt.serialize().unwrap();
    assert!(inspect_ironwood(&bytes, &fixture.terms, &support::secret(22)).is_err());
    let mut changed = support::input();
    changed.window = fixture.terms.window();
    // Validly encoded but different recipient must not reuse the prepared payment.
    let changed = OrderTerms::new(changed).unwrap();
    assert!(inspect_ironwood(&bytes, &changed, &fixture.randomness).is_err());
    assert!(inspect_ironwood(b"not a PCZT", &fixture.terms, &fixture.randomness).is_err());
}

#[cfg(feature = "native")]
#[test]
fn source_note_archive_builds_a_bound_payment_and_rejects_wrong_ownership_or_anchor() {
    use incrementalmerkletree::{Hashable, Level};
    use orchard::{
        Note,
        keys::FullViewingKey,
        note::{ExtractedNoteCommitment, NoteVersion, RandomSeed, Rho},
        tree::{MerkleHashOrchard, MerklePath},
        value::NoteValue,
    };
    use zutti_protocol::encoding::Canonical;
    use zutti_wallet::source_notes::{SourceNote, SourceScan};
    let fixture = fixture(false);
    let viewing = FullViewingKey::from(&fixture.key);
    let rho = Rho::from_bytes(&[1; 32]).into_option().unwrap();
    let seed = (0..=255_u8)
        .find_map(|n| RandomSeed::from_bytes([n; 32], &rho).into_option())
        .unwrap();
    let note = Note::from_parts(
        viewing.address_at(0_u32, orchard::keys::Scope::External),
        NoteValue::from_raw(500_010_000),
        rho,
        seed,
        NoteVersion::V3,
    )
    .into_option()
    .unwrap();
    let path = MerklePath::from_parts(
        0,
        std::array::from_fn(|i| MerkleHashOrchard::empty_root(Level::from(i as u8))),
    );
    let anchor = path
        .root(ExtractedNoteCommitment::from(note.commitment()))
        .to_bytes();
    let scan = SourceScan {
        height: fixture.terms.window().anchor_height(),
        block: fixture.terms.window().anchor_hash(),
        anchor,
        notes: vec![SourceNote {
            note,
            path,
            txid: support::digest(101),
            action: 0,
            received_height: 4_460_000,
        }],
    };
    let restored = SourceScan::from_bytes(&scan.to_bytes().unwrap()).unwrap();
    let pczt = restored
        .prepare_payment(&viewing, &fixture.terms, &fixture.randomness)
        .unwrap();
    inspect_ironwood(&pczt, &fixture.terms, &fixture.randomness).unwrap();
    let other = orchard::keys::SpendingKey::from_bytes([99; 32]).unwrap();
    assert!(
        restored
            .prepare_payment(
                &FullViewingKey::from(&other),
                &fixture.terms,
                &fixture.randomness
            )
            .is_err()
    );
    let mut wrong = scan;
    wrong.anchor[0] ^= 1;
    assert!(SourceScan::from_bytes(&wrong.to_bytes().unwrap()).is_err());
    assert!(
        wrong
            .prepare_payment(&viewing, &fixture.terms, &fixture.randomness)
            .is_err()
    );
    wrong.anchor[0] ^= 1;
    wrong.block.0[0] ^= 1;
    assert!(
        wrong
            .prepare_payment(&viewing, &fixture.terms, &fixture.randomness)
            .is_err()
    );
}

#[cfg(all(feature = "native", feature = "spv-pool"))]
#[test]
fn prepared_order_binds_owned_receivers_and_survives_restore() {
    #[path = "../../../tests/support/spv_pool.rs"]
    mod pool_fixture;
    use incrementalmerkletree::{Hashable, Level};
    use orchard::{
        Note,
        keys::Scope,
        note::{ExtractedNoteCommitment, NoteVersion, RandomSeed, Rho},
        tree::{MerkleHashOrchard, MerklePath},
        value::NoteValue,
    };
    use zutti_protocol::{
        amount::{Amount, Asset, FeeBps},
        domain::Domain,
        encoding::Canonical,
    };
    use zutti_source::spv::Policy;
    use zutti_wallet::{
        account::WalletAccount,
        order::{MakerRecipients, OrderContext, PreparedOrder, QuoteTerms},
        source_notes::{SourceNote, SourceScan},
    };
    let root = tempfile::tempdir().unwrap();
    let taker = WalletAccount::create(&root.path().join("taker")).unwrap();
    let maker = WalletAccount::create(&root.path().join("maker")).unwrap();
    let initial = Policy::new(
        support::digest(51),
        zcash_protocol::consensus::NetworkType::Test,
        10,
        128,
        5_000_000,
    )
    .unwrap();
    let checkpoint = pool_fixture::unverified_anchor(initial);
    let policy = Policy::from_checkpoint(
        initial.program(),
        10,
        128,
        5_000_000,
        checkpoint.checkpoint_id().unwrap(),
    )
    .unwrap();
    let anchor = zutti_source::spv::HeaderClaim::from_checkpoint(policy, &checkpoint).unwrap();
    let domain = Domain::new(
        zutti_protocol::network::ZCASH_TESTNET_GENESIS,
        11155111,
        [2; 20],
        [3; 20],
        zutti_wallet::binding::payment_profile(),
        policy.id().unwrap(),
        support::digest(6),
    )
    .unwrap();
    let rho = Rho::from_bytes(&[1; 32]).into_option().unwrap();
    let seed = (0..=255_u8)
        .find_map(|n| RandomSeed::from_bytes([n; 32], &rho).into_option())
        .unwrap();
    let note = Note::from_parts(
        taker
            .source_viewing_key()
            .address_at(0_u32, Scope::External),
        NoteValue::from_raw(100_000_000),
        rho,
        seed,
        NoteVersion::V3,
    )
    .into_option()
    .unwrap();
    let path = MerklePath::from_parts(
        0,
        std::array::from_fn(|i| MerkleHashOrchard::empty_root(Level::from(i as u8))),
    );
    let scan = SourceScan {
        height: anchor.height().unwrap(),
        block: anchor.tip().unwrap(),
        anchor: path
            .root(ExtractedNoteCommitment::from(note.commitment()))
            .to_bytes(),
        notes: vec![SourceNote {
            note,
            path,
            txid: support::digest(90),
            action: 0,
            received_height: anchor.height().unwrap(),
        }],
    };
    let prepare = || {
        PreparedOrder::prepare(
            OrderContext {
                domain: domain.clone(),
                policy,
                anchor: anchor.clone(),
                source: &scan,
                account: &taker,
            },
            QuoteTerms {
                offer: support::digest(91),
                zec: Amount::parse(Asset::Zec, "0.01").unwrap(),
                gross_usdc: Amount::parse(Asset::Usdc, "1").unwrap(),
                fee_bps: FeeBps::new(50).unwrap(),
                fee_caps: [
                    Amount::parse(Asset::Zec, "0.001").unwrap(),
                    Amount::parse(Asset::Eth, "0.005").unwrap(),
                    Amount::from_units(Asset::Usdc, 0).unwrap(),
                ],
                nonce: support::digest(92),
            },
            MakerRecipients::from_address(
                &maker.zcash_address().unwrap(),
                maker.private_receiver().unwrap(),
            )
            .unwrap(),
        )
    };
    let order = prepare().unwrap();
    let reservation = order.reservation();
    assert_eq!(reservation.terms.receiver(), &maker.source_receiver());
    assert_eq!(
        reservation.payout.note.recipient(),
        taker.private_receiver().unwrap()
    );
    assert_eq!(
        reservation.refund.note.recipient(),
        maker.private_receiver().unwrap()
    );
    assert_eq!(reservation.terms.usdc().units(), 995_000);
    assert_eq!(reservation.terms.window().expiry(), scan.height + 128);
    let bytes = order.to_bytes().unwrap();
    assert_eq!(taker.sign_payment(order.payment()).unwrap().len(), 1);
    assert!(maker.sign_payment(order.payment()).is_err());
    let restored = PreparedOrder::from_bytes(&bytes).unwrap();
    assert_eq!(restored.payment(), order.payment());
    assert_eq!(
        restored.binding_witness().unwrap().evaluate().unwrap(),
        reservation.binding
    );
    assert_ne!(prepare().unwrap().reservation().txid, reservation.txid);
    let mut changed = bytes.clone();
    changed.push(0);
    assert!(PreparedOrder::from_bytes(&changed).is_err());
    assert!(PreparedOrder::from_bytes(&bytes[..bytes.len() - 1]).is_err());
    assert!(
        MakerRecipients::from_address("utest1invalid", maker.private_receiver().unwrap()).is_err()
    );
}
