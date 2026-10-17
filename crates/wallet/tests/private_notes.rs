#![cfg(feature = "native")]

use zutti_protocol::{
    Digest, Result, Secret32,
    amount::{Amount, Asset},
    encoding::Canonical,
    private_note::{NoteRecipient, PrivateNote, SpendAuthorization},
};
use zutti_wallet::private_note::{
    EncryptedNote, NoteEncryption, NoteKeys, decrypt_with_viewing_key,
};

fn keys(value: u8) -> Result<NoteKeys> {
    NoteKeys::from_secrets(
        Secret32::from_bytes([value; 32])?,
        Secret32::from_bytes([value + 1; 32])?,
        Secret32::from_bytes([value + 2; 32])?,
    )
}

fn note(keys: &NoteKeys, domain: u8, rho: u8) -> Result<PrivateNote> {
    PrivateNote::new(
        Digest([domain; 32]),
        keys.recipient()?,
        Amount::from_units(Asset::Usdc, 200_000_000)?,
        Secret32::from_bytes([rho; 32])?,
        Secret32::from_bytes([9; 32])?,
    )
}

#[test]
fn viewing_recovery_does_not_grant_spending_authority() -> Result<()> {
    let owner = keys(1)?;
    let note = note(&owner, 3, 4)?;
    let encrypted = EncryptedNote::encrypt(&note)?;
    let recovered =
        decrypt_with_viewing_key(&encrypted, &Secret32::from_bytes([2; 32])?, Digest([3; 32]))?;
    assert_eq!(recovered.commitment()?, note.commitment()?);
    assert_eq!(owner.nullifier(&recovered)?, owner.nullifier(&note)?);
    assert!(
        recovered
            .nullifier(&Secret32::from_bytes([2; 32])?)
            .is_err()
    );
    assert_eq!(recovered.amount().units(), 200_000_000);
    let operation = Digest([5; 32]);
    let signature = owner.authorize(&recovered, operation)?;
    recovered.verify_spend(operation, &signature)?;
    assert!(recovered.verify_spend(Digest([6; 32]), &signature).is_err());
    assert!(keys(7)?.authorize(&recovered, operation).is_err());
    assert!(format!("{owner:?}{recovered:?}{signature:?}").contains("REDACTED"));
    Ok(())
}

#[test]
fn note_authorization_is_bound_to_domain_and_exact_note() -> Result<()> {
    let owner = keys(1)?;
    let original = note(&owner, 3, 4)?;
    let operation = Digest([5; 32]);
    let signature = owner.authorize(&original, operation)?;
    let parsed = k256::ecdsa::Signature::from_slice(signature.as_bytes())
        .map_err(|_| zutti_protocol::Error::Invalid("test signature"))?;
    let (r, s) = parsed.split_scalars();
    let high_s = k256::ecdsa::Signature::from_scalars(r.to_bytes(), (-s).to_bytes())
        .map_err(|_| zutti_protocol::Error::Invalid("test signature"))?;
    assert!(SpendAuthorization::from_bytes(high_s.to_bytes().into()).is_err());
    assert!(
        note(&owner, 6, 4)?
            .verify_spend(operation, &signature)
            .is_err()
    );
    assert!(
        note(&owner, 3, 7)?
            .verify_spend(operation, &signature)
            .is_err()
    );
    assert!(original.spend_digest(Digest([0; 32])).is_err());
    let mut altered = *signature.as_bytes();
    altered[0] ^= 1;
    if let Ok(altered) = SpendAuthorization::from_bytes(altered) {
        assert!(original.verify_spend(operation, &altered).is_err());
    }
    assert!(SpendAuthorization::from_bytes([0; 64]).is_err());
    Ok(())
}

#[test]
fn ciphertext_metadata_and_plaintext_are_authenticated() -> Result<()> {
    let owner = keys(1)?;
    let original = note(&owner, 3, 4)?;
    let encrypted = EncryptedNote::encrypt(&original)?;
    let encoded = encrypted.to_bytes()?;
    for offset in [10, 42, 74, 106, 134, encoded.len() - 1] {
        let mut altered = encoded.clone();
        altered[offset] ^= 1;
        if let Ok(altered) = EncryptedNote::from_bytes(&altered) {
            assert!(owner.decrypt(&altered, original.domain()).is_err());
        }
    }
    assert!(owner.decrypt(&encrypted, Digest([6; 32])).is_err());
    assert!(keys(7)?.decrypt(&encrypted, original.domain()).is_err());
    let mut trailing = encoded.clone();
    trailing.push(0);
    assert!(EncryptedNote::from_bytes(&trailing).is_err());
    assert!(EncryptedNote::from_bytes(&encoded[..encoded.len() - 1]).is_err());
    assert_eq!(EncryptedNote::from_bytes(&encoded)?, encrypted);
    Ok(())
}

#[test]
fn fresh_encryption_preserves_the_committed_note() -> Result<()> {
    let owner = keys(1)?;
    let original = note(&owner, 3, 4)?;
    let first = EncryptedNote::encrypt(&original)?;
    let second = EncryptedNote::encrypt(&original)?;
    assert_ne!(first, second);
    assert_eq!(first.commitment(), second.commitment());
    assert_eq!(
        owner.decrypt(&second, original.domain())?.commitment()?,
        original.commitment()?
    );
    let fixed = first.fixed_output(&original)?;
    assert_eq!(fixed.commitment(), original.commitment()?);
    assert_eq!(fixed.ciphertext_digest(), first.digest()?);
    assert!(first.fixed_output(&note(&owner, 3, 7)?).is_err());
    assert_ne!(
        owner.nullifier(&note(&owner, 3, 7)?)?,
        owner.nullifier(&original)?
    );
    Ok(())
}

#[test]
fn encryption_randomness_can_be_checked_without_a_spending_key() -> Result<()> {
    let owner = keys(1)?;
    let original = note(&owner, 3, 4)?;
    let randomness = || -> Result<_> {
        Ok(NoteEncryption::from_entropy(
            Secret32::from_bytes([8; 32])?,
            [9; 24],
        ))
    };
    let first = EncryptedNote::encrypt_with(&original, randomness()?)?;
    let recomputed = EncryptedNote::encrypt_with(&original, randomness()?)?;
    assert_eq!(first, recomputed);
    let recovered = owner.decrypt(&first, original.domain())?;
    assert_eq!(recovered.commitment()?, original.commitment()?);
    Ok(())
}

#[test]
fn invalid_keys_and_non_usdc_notes_are_rejected() -> Result<()> {
    let owner = keys(1)?;
    assert!(
        NoteKeys::from_secrets(
            Secret32::from_bytes([255; 32])?,
            Secret32::from_bytes([2; 32])?,
            Secret32::from_bytes([3; 32])?,
        )
        .is_err()
    );
    assert!(NoteRecipient::new([0; 33], [2; 32], Digest([3; 32])).is_err());
    assert!(NoteRecipient::new(*owner.recipient()?.owner(), [255; 32], Digest([3; 32])).is_err());
    assert!(NoteRecipient::new(*owner.recipient()?.owner(), [0; 32], Digest([3; 32])).is_err());
    for amount in [
        Amount::from_units(Asset::Usdc, 0)?,
        Amount::from_units(Asset::Zec, 1)?,
    ] {
        assert!(
            PrivateNote::new(
                Digest([3; 32]),
                owner.recipient()?,
                amount,
                Secret32::from_bytes([4; 32])?,
                Secret32::from_bytes([9; 32])?,
            )
            .is_err()
        );
    }
    let mut low_order = [0; 32];
    low_order[0] = 1;
    let unreceivable = PrivateNote::new(
        Digest([3; 32]),
        NoteRecipient::new(*owner.recipient()?.owner(), low_order, Digest([3; 32]))?,
        Amount::from_units(Asset::Usdc, 1)?,
        Secret32::from_bytes([4; 32])?,
        Secret32::from_bytes([9; 32])?,
    )?;
    assert!(EncryptedNote::encrypt(&unreceivable).is_err());
    Ok(())
}

#[test]
fn note_codecs_reject_noncanonical_and_truncated_inputs() -> Result<()> {
    let owner = keys(1)?;
    let original = note(&owner, 3, 4)?;
    let mut bytes = original.to_bytes()?;
    assert_eq!(
        PrivateNote::from_bytes(&bytes)?.commitment()?,
        original.commitment()?
    );
    bytes.push(0);
    assert!(PrivateNote::from_bytes(&bytes).is_err());
    bytes.pop();
    bytes[8] = 2;
    assert!(PrivateNote::from_bytes(&bytes).is_err());
    Ok(())
}

#[test]
fn fixed_output_ciphertext_digest_matches_the_ethereum_journal() -> Result<()> {
    use sha2::Digest as _;
    let owner = keys(1)?;
    let original = note(&owner, 3, 4)?;
    let encrypted = EncryptedNote::encrypt_with(
        &original,
        NoteEncryption::from_entropy(Secret32::from_bytes([41; 32])?, [42; 24]),
    )?;
    let fixed = encrypted.fixed_output(&original)?;
    assert_eq!(
        fixed.ciphertext_digest().0,
        <[u8; 32]>::from(sha2::Sha256::digest(encrypted.to_bytes()?))
    );
    Ok(())
}
