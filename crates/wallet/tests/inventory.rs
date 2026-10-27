#![cfg(all(feature = "native", feature = "spv-pool"))]
use zutti_protocol::{
    Digest, Result, Secret32,
    amount::{Amount, Asset},
    encoding::Canonical,
    pool::PoolTree,
    private_note::PrivateNote,
};
use zutti_wallet::{
    inventory::{PoolInventory, PoolOutput},
    pool::escrow::EscrowNote,
    private_note::{EncryptedNote, NoteKeys},
};

fn keys(value: u8) -> Result<NoteKeys> {
    NoteKeys::from_secrets(
        Secret32::from_bytes([value; 32])?,
        Secret32::from_bytes([value + 1; 32])?,
        Secret32::from_bytes([value + 2; 32])?,
    )
}

#[test]
fn mixed_pool_history_retains_escrow_membership_and_recovers_the_owned_payout()
-> zutti_wallet::Result<()> {
    let domain = Digest([3; 32]);
    let owner = keys(10)?;
    let foreign = keys(20)?;
    let note = |keys: &NoteKeys, rho| -> Result<_> {
        let note = PrivateNote::new(
            domain,
            keys.recipient()?,
            Amount::from_units(Asset::Usdc, 1_000_000)?,
            Secret32::from_bytes([rho; 32])?,
            Secret32::from_bytes([40; 32])?,
        )?;
        EncryptedNote::encrypt(&note)
    };
    let first = note(&foreign, 30)?;
    let last = note(&owner, 31)?;
    let escrow = EscrowNote::new(
        domain,
        Digest([4; 32]),
        Digest([5; 32]),
        1_000_000,
        Secret32::from_bytes([6; 32])?,
        Secret32::from_bytes([7; 32])?,
    )?;
    let envelope = escrow.encrypt([8; 24])?;
    let encoded = [first.to_bytes()?, envelope.clone(), last.to_bytes()?];
    let outputs: Vec<_> = encoded
        .iter()
        .enumerate()
        .map(|(position, bytes)| Ok((position as u32, PoolOutput::from_bytes(bytes)?)))
        .collect::<zutti_wallet::Result<_>>()?;
    let mut tree = PoolTree::new(domain)?;
    for (_, output) in &outputs {
        tree.append(output.commitment())?;
    }
    let inventory = PoolInventory::scan_outputs(domain, &owner, &outputs, tree.root(), 3)?;
    assert_eq!(inventory.notes().len(), 1);
    assert_eq!(inventory.notes()[0].path.position, 2);
    assert_eq!(inventory.notes()[0].note.amount().units(), 1_000_000);
    inventory.notes()[0]
        .path
        .verify(domain, last.commitment(), tree.root())?;
    assert!(PoolInventory::scan_outputs(domain, &owner, &outputs[..2], tree.root(), 3).is_err());
    assert!(
        PoolInventory::scan_outputs(Digest([9; 32]), &owner, &outputs, tree.root(), 3).is_err()
    );
    for bytes in [
        envelope[..envelope.len() - 1].to_vec(),
        [envelope.as_slice(), &[0]].concat(),
    ] {
        assert!(PoolOutput::from_bytes(&bytes).is_err());
    }
    let mut changed = envelope;
    changed[8] = 2;
    assert!(PoolOutput::from_bytes(&changed).is_err());
    Ok(())
}
