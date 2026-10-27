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
