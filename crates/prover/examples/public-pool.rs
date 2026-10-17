use std::path::{Path, PathBuf};
use zutti_protocol::{
    Digest, Secret32,
    amount::{Amount, Asset},
    domain::Domain,
    encoding::Canonical,
    pool::{OperationKind, PoolClaim, PoolTree, nullifier_digest, output_digest},
    private_note::PrivateNote,
};
use zutti_prover::pool::{pool_image_id, pool_statement};
use zutti_wallet::{
    backup,
    pool::{NoteInput, NoteOutput, PoolAction, PoolWitness},
    private_note::{NoteEncryption, NoteKeys},
};

fn secret(byte: u8) -> Result<Secret32, zutti_protocol::Error> {
    Secret32::from_bytes([byte; 32])
}
fn keys(byte: u8) -> Result<NoteKeys, zutti_protocol::Error> {
    NoteKeys::from_secrets(secret(byte)?, secret(byte + 1)?, secret(byte + 2)?)
}
fn output(
    domain: Digest,
    keys: &NoteKeys,
    amount: u128,
    nonce: u8,
) -> Result<NoteOutput, zutti_protocol::Error> {
    Ok(NoteOutput {
        note: PrivateNote::new(
            domain,
            keys.recipient()?,
            Amount::from_units(Asset::Usdc, amount)?,
            secret(nonce)?,
            secret(nonce + 1)?,
        )?,
        encryption: NoteEncryption::from_entropy(secret(nonce + 2)?, [nonce + 3; 24]),
    })
}
fn address(value: &serde_json::Value, key: &str) -> Result<[u8; 20], Box<dyn std::error::Error>> {
    let value = value[key].as_str().ok_or("missing address")?;
    Ok(hex::decode(value.strip_prefix("0x").unwrap_or(value))?
        .try_into()
        .map_err(|_| "invalid address")?)
}
fn digest(value: &serde_json::Value, key: &str) -> Result<Digest, Box<dyn std::error::Error>> {
    let text = value[key].as_str().ok_or("missing digest")?;
    Ok(Digest::from_hex(text.strip_prefix("0x").unwrap_or(text))?)
}

fn claim(
    kind: OperationKind,
    domain: Digest,
    root: Digest,
    value: u64,
    owner: [u8; 20],
    inputs: &[Digest],
    outputs: &[(Digest, Digest)],
) -> Result<PoolClaim, zutti_protocol::Error> {
    Ok(PoolClaim {
        kind,
        domain,
        root,
        checkpoint: Digest::default(),
        order_tag: Digest::default(),
        order: Digest::default(),
        reservation: Digest::default(),
        value,
        owner,
        nullifiers: nullifier_digest(inputs)?,
        outputs: output_digest(outputs)?,
    })
}
fn export(witness: PoolWitness, directory: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let claim = witness.evaluate()?;
    let statement = pool_statement(&claim)?;
    std::fs::create_dir(directory)?;
    let directory = directory.canonicalize()?;
    let witness_bytes = witness.to_bytes()?;
    let mut stdin = (witness_bytes.len() as u32).to_le_bytes().to_vec();
    stdin.extend_from_slice(&witness_bytes);
    backup::persist_new(&directory.join("program.elf"), zutti_methods::POOL_ELF)?;
    backup::persist_new(&directory.join("stdin.bin"), &stdin)?;
    backup::persist_new(&directory.join("witness.bin"), &witness_bytes)?;
    backup::persist_new(&directory.join("journal.bin"), statement.payload())?;
    backup::persist_new(&directory.join("statement.bin"), &statement.to_bytes()?)?;
    let nullifiers = witness
        .inputs
        .iter()
        .map(|input| {
            input
                .note
                .nullifier(&input.nullifier_key)
                .map(|n| n.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let outputs = witness.outputs.iter().map(|output| Ok(serde_json::json!({"commitment":output.note.commitment()?.to_string(),"ciphertext":hex::encode(output.encrypted()?.to_bytes()?)}))).collect::<Result<Vec<_>, zutti_protocol::Error>>()?;
    backup::persist_new(
        &directory.join("operation.json"),
        &serde_json::to_vec_pretty(
            &serde_json::json!({"kind":claim.kind as u8,"domain":claim.domain.to_string(),"root":claim.root.to_string(),"checkpoint":claim.checkpoint.to_string(),"order_tag":claim.order_tag.to_string(),"order":claim.order.to_string(),"reservation":claim.reservation.to_string(),"value":claim.value,"owner":hex::encode(claim.owner),"nullifiers":nullifiers,"outputs":outputs}),
        )?,
    )?;
    backup::persist_new(
        &directory.join("job.json"),
        &serde_json::to_vec_pretty(
            &serde_json::json!({"version":1,"visibility":"public","image_id":pool_image_id().to_string(),"program":directory.join("program.elf"),"stdin":directory.join("stdin.bin"),"journal":directory.join("journal.bin"),"cycles":70_000_000}),
        )?,
    )?;
    Ok(())
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("usage: public-pool LOCAL_DEPLOYMENT_JSON OUTPUT_DIRECTORY".into());
    }
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(PathBuf::from(&args[0]))?)?;
    if config["chain_id"] != 31337 {
        return Err("public note-key demonstrations require the local chain".into());
    }
    let domain = Domain::new(
        digest(&config, "source_genesis")?,
        31337,
        address(&config, "pool")?,
        address(&config, "token")?,
        digest(&config, "payment_profile")?,
        digest(&config, "source_policy")?,
        digest(&config, "proof_profile")?,
    )?
    .id()?;
    let owner_address = address(&config, "owner")?;
    let directory = PathBuf::from(&args[1]);
    std::fs::create_dir(&directory)?;
    let owner = keys(9)?;
    let receiver = keys(29)?;
    let initial = output(domain, &owner, 200_000_000, 49)?;
    let mut tree = PoolTree::new(domain)?;
    tree.append(initial.note.commitment()?)?;
    let public = claim(
        OperationKind::Deposit,
        domain,
        Digest::default(),
        200_000_000,
        owner_address,
        &[],
        &[initial.binding()?],
    )?;
    export(
        PoolWitness {
            claim: public,
            inputs: vec![],
            outputs: vec![initial],
            action: PoolAction::Deposit,
        },
        &directory.join("deposit"),
    )?;
    let initial = output(domain, &owner, 200_000_000, 49)?;
    let receiver_note = output(domain, &receiver, 150_000_000, 69)?;
    let change = output(domain, &owner, 50_000_000, 89)?;
    let public = claim(
        OperationKind::Transfer,
        domain,
        tree.root(),
        0,
        [0; 20],
        &[owner.nullifier(&initial.note)?],
        &[receiver_note.binding()?, change.binding()?],
    )?;
    let input = NoteInput::authorize(
        initial.note,
        tree.prove(0)?,
        &owner,
        public.journal_digest()?,
    )?;
    tree.append(receiver_note.note.commitment()?)?;
    tree.append(change.note.commitment()?)?;
    export(
        PoolWitness {
            claim: public,
            inputs: vec![input],
            outputs: vec![receiver_note, change],
            action: PoolAction::Transfer,
        },
        &directory.join("transfer"),
    )?;
    let receiver_note = output(domain, &receiver, 150_000_000, 69)?;
    let change = output(domain, &receiver, 50_000_000, 109)?;
    let public = claim(
        OperationKind::Withdraw,
        domain,
        tree.root(),
        100_000_000,
        owner_address,
        &[receiver.nullifier(&receiver_note.note)?],
        &[change.binding()?],
    )?;
    let input = NoteInput::authorize(
        receiver_note.note,
        tree.prove(1)?,
        &receiver,
        public.journal_digest()?,
    )?;
    export(
        PoolWitness {
            claim: public,
            inputs: vec![input],
            outputs: vec![change],
            action: PoolAction::Withdraw,
        },
        &directory.join("withdraw"),
    )?;
    println!(
        "{}",
        serde_json::json!({"visibility":"generated_public_demo","network":"local","directory":directory,"domain":domain.to_string(),"pool_image":pool_image_id().to_string()})
    );
    Ok(())
}
