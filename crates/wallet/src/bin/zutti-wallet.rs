use clap::{Parser, Subcommand};
use std::{fs, io::Read, path::PathBuf};
use zutti_protocol::{
    Digest, Secret32,
    amount::{Amount, Asset},
    domain::Domain,
    encoding::Canonical,
    network::ZCASH_TESTNET_GENESIS,
};
use zutti_source::spv::Policy;
use zutti_wallet::{account::WalletAccount, backup, deposit::DepositContext};

#[derive(Parser)]
#[command(name = "zutti-wallet")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Init {
        #[arg(long)]
        directory: PathBuf,
    },
    Address {
        #[arg(long)]
        directory: PathBuf,
    },
    Deposit {
        #[arg(long)]
        directory: PathBuf,
        #[arg(long)]
        deployment: PathBuf,
        #[arg(long)]
        amount: String,
        #[arg(long)]
        out: PathBuf,
    },
    Note {
        #[arg(long)]
        directory: PathBuf,
        #[arg(long)]
        deployment: PathBuf,
        #[arg(long)]
        ciphertext: PathBuf,
    },
    Inventory {
        #[arg(long)]
        directory: PathBuf,
        #[arg(long)]
        deployment: PathBuf,
        #[arg(long)]
        events: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    Scan {
        #[arg(long)]
        directory: PathBuf,
        #[arg(long)]
        trees: PathBuf,
        #[arg(long)]
        block: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    PreparePayment {
        #[arg(long)]
        directory: PathBuf,
        #[arg(long)]
        deployment: PathBuf,
        #[arg(long)]
        notes: PathBuf,
        #[arg(long)]
        terms: PathBuf,
        #[arg(long)]
        randomness: PathBuf,
        #[arg(long)]
        reservation_randomness: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    #[cfg(feature = "wallet-proving")]
    ProvePayment {
        #[arg(long)]
        pczt: PathBuf,
        #[arg(long)]
        terms: PathBuf,
        #[arg(long)]
        randomness: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let account = match Args::parse().command {
        Command::Init { directory } => WalletAccount::create(&directory)?,
        Command::Address { directory } => WalletAccount::open(&directory)?,
        Command::Deposit {
            directory,
            deployment,
            amount,
            out,
        } => {
            return deposit(directory, deployment, amount, out);
        }
        Command::Note {
            directory,
            deployment,
            ciphertext,
        } => {
            let profile = read_deployment(deployment)?;
            let domain = Digest::from_hex(profile["domain"].as_str().ok_or("missing domain")?)?;
            let account = WalletAccount::open(&directory)?;
            let mut bytes = Vec::new();
            fs::File::open(ciphertext)?
                .take(4097)
                .read_to_end(&mut bytes)?;
            if bytes.len() > 4096 {
                return Err("note ciphertext exceeds its bound".into());
            }
            let encrypted = zutti_wallet::private_note::EncryptedNote::from_bytes(&bytes)?;
            let note = account.note_keys().decrypt(&encrypted, domain)?;
            if note.recipient() != account.private_receiver()? {
                return Err("note ownership mismatch".into());
            }
            println!(
                "{}",
                serde_json::json!({"owned":true,"amount_units":note.amount().units(),"commitment":note.commitment()?.to_string(),"membership_verified":false})
            );
            return Ok(());
        }
        Command::Inventory {
            directory,
            deployment,
            events,
            out,
        } => {
            return inventory(directory, deployment, events, out);
        }
        Command::Scan {
            directory,
            trees,
            block,
            out,
        } => return scan(directory, trees, block, out),
        Command::PreparePayment {
            directory,
            deployment,
            notes,
            terms,
            randomness,
            reservation_randomness,
            out,
        } => {
            return prepare_payment(
                directory,
                deployment,
                notes,
                terms,
                randomness,
                reservation_randomness,
                out,
            );
        }
        #[cfg(feature = "wallet-proving")]
        Command::ProvePayment {
            pczt,
            terms,
            randomness,
            out,
        } => {
            if out.try_exists()? {
                return Err("proven payment output already exists".into());
            }
            let pczt = backup::read_private(&pczt, zutti_wallet::prepare::MAX_PCZT)?;
            let terms = zutti_protocol::order::OrderTerms::from_bytes(&backup::read_private(
                &terms, 4096,
            )?)?;
            let randomness = backup::read_private(&randomness, 32)?;
            let randomness = Secret32::from_bytes(randomness.as_slice().try_into()?)?;
            let proven = zeroize::Zeroizing::new(zutti_wallet::source_notes::prove_payment(
                &pczt,
                &terms,
                &randomness,
            )?);
            backup::persist_new(&out, &proven)?;
            println!(
                "{}",
                serde_json::json!({"native_proof_generated":true,"authorized":false,"broadcast":false})
            );
            return Ok(());
        }
    };
    let recipient = account.private_receiver()?;
    println!(
        "{}",
        serde_json::json!({
            "version": 1,
            "network": "testnet",
            "zcash_address": account.zcash_address()?,
            "usdc_receiver": {
                "owner": hex_bytes(recipient.owner()),
                "viewing": hex_bytes(recipient.viewing()),
                "nullifier_key_commitment": recipient.nullifier_key().to_string()
            }
        })
    );
    Ok(())
}

fn prepare_payment(
    directory: PathBuf,
    deployment: PathBuf,
    notes: PathBuf,
    terms: PathBuf,
    randomness: PathBuf,
    reservation_randomness: PathBuf,
    out: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    use zutti_wallet::{
        binding::PaymentBindingWitness, preparation::PreparationWitness, source_notes::SourceScan,
    };
    if out.try_exists()? {
        return Err("payment output already exists".into());
    }
    let (domain, policy) = deployment_context(&read_deployment(deployment)?)?;
    let account = WalletAccount::open(&directory)?;
    let key = backup::read_private(&directory.join("account.key"), 32)?;
    let key = Secret32::from_bytes(key.as_slice().try_into()?)?;
    let archive = backup::read_private(&notes, 262_144)?;
    let scan = SourceScan::from_bytes(&backup::open(ZCASH_TESTNET_GENESIS, &archive, &key)?)?;
    let terms =
        zutti_protocol::order::OrderTerms::from_bytes(&backup::read_private(&terms, 4096)?)?;
    if terms.domain() != &domain {
        return Err("payment deployment mismatch".into());
    }
    let randomness = backup::read_private(&randomness, 32)?;
    let randomness = Secret32::from_bytes(randomness.as_slice().try_into()?)?;
    let reservation_randomness = backup::read_private(&reservation_randomness, 32)?;
    let reservation_randomness =
        Secret32::from_bytes(reservation_randomness.as_slice().try_into()?)?;
    let pczt = zeroize::Zeroizing::new(scan.prepare_payment(
        &account.source_viewing_key(),
        &terms,
        &randomness,
    )?);
    let inspected = zutti_wallet::prepare::inspect_ironwood(&pczt, &terms, &randomness)?;
    let binding = PaymentBindingWitness::new(
        policy,
        PreparationWitness::new(terms, randomness, reservation_randomness, pczt.to_vec())?,
    )?;
    let claim = binding.evaluate()?;
    let parent = out
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    let stage = tempfile::Builder::new()
        .prefix(".zutti-payment-")
        .tempdir_in(parent)?;
    backup::persist_new(&stage.path().join("unsigned.pczt"), &pczt)?;
    backup::persist_new(
        &stage.path().join("payment.backup"),
        &backup::seal(domain.id()?, &pczt, &key)?,
    )?;
    backup::persist_new(
        &stage.path().join("binding-witness.bin"),
        &binding.to_bytes()?,
    )?;
    backup::persist_new(&stage.path().join("binding-claim.bin"), &claim.to_bytes()?)?;
    let summary = serde_json::json!({"txid":inspected.txid().to_string(),"expiry":inspected.expiry(),"order":claim.order().to_string(),"reservation":claim.reservation().to_string(),"authorized":false,"broadcast":false});
    backup::persist_new(
        &stage.path().join("payment.json"),
        &serde_json::to_vec_pretty(&summary)?,
    )?;
    fs::rename(stage.path(), &out)?;
    fs::File::open(parent)?.sync_all()?;
    println!("{summary}");
    Ok(())
}

fn scan(
    directory: PathBuf,
    trees: PathBuf,
    block: PathBuf,
    out: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    use zutti_protocol::encoding::Writer;
    if out.try_exists()? {
        return Err("scan output already exists".into());
    }
    let account = WalletAccount::open(&directory)?;
    let mut data = Vec::new();
    fs::File::open(trees)?.take(16_385).read_to_end(&mut data)?;
    if data.len() > 16_384 {
        return Err("tree state exceeds its bound".into());
    }
    let trees: serde_json::Value = serde_json::from_slice(&data)?;
    let previous = &trees["previous"];
    let current = &trees["current"];
    let height = u32::try_from(
        previous["height"]
            .as_u64()
            .ok_or("missing previous height")?,
    )?;
    if current["height"].as_u64() != Some(u64::from(height) + 1) {
        return Err("tree state height mismatch".into());
    }
    let mut parent = hex::decode(previous["hash"].as_str().ok_or("missing previous hash")?)?;
    parent.reverse();
    let parent = Digest::nonzero(parent.try_into().map_err(|_| "invalid block hash")?)?;
    let tree = hex::decode(
        previous["ironwood"]["commitments"]["finalState"]
            .as_str()
            .ok_or("missing Ironwood tree")?,
    )?;
    let anchor = hex::decode(
        current["ironwood"]["commitments"]["finalRoot"]
            .as_str()
            .ok_or("missing Ironwood anchor")?,
    )?;
    let mut block_data = Vec::new();
    fs::File::open(block)?
        .take(2_000_001)
        .read_to_end(&mut block_data)?;
    if block_data.len() > 2_000_000 {
        return Err("block exceeds its bound".into());
    }
    let scan = zutti_wallet::source_notes::scan_ironwood(
        &account.source_viewing_key(),
        height,
        parent,
        &tree,
        &[block_data],
        anchor.try_into().map_err(|_| "invalid anchor")?,
    )?;
    if scan.block.to_string()
        != hex::encode(
            hex::decode(current["hash"].as_str().ok_or("missing block hash")?)?
                .into_iter()
                .rev()
                .collect::<Vec<_>>(),
        )
    {
        return Err("scan block hash mismatch".into());
    }
    let mut writer = Writer::default();
    writer.fixed(b"ZUTTISCN");
    writer.u16(1);
    writer.u32(scan.height);
    writer.fixed(&scan.block.0);
    writer.fixed(&scan.anchor);
    writer.u32(u32::try_from(scan.notes.len())?);
    let mut balance = 0_u64;
    for note in &scan.notes {
        note.encode(&mut writer)?;
        balance = balance
            .checked_add(note.note.value().inner())
            .ok_or("balance overflow")?;
    }
    let key = backup::read_private(&directory.join("account.key"), 32)?;
    let key = Secret32::from_bytes(key.as_slice().try_into()?)?;
    let plaintext = zeroize::Zeroizing::new(writer.finish());
    let archive = backup::seal(ZCASH_TESTNET_GENESIS, &plaintext, &key)?;
    backup::persist_new(&out, &archive)?;
    println!(
        "{}",
        serde_json::json!({"height":scan.height,"block":scan.block.to_string(),"ironwood_notes":scan.notes.len(),"amount_units":balance,"transaction_validity_verified":false})
    );
    Ok(())
}

fn inventory(
    directory: PathBuf,
    deployment: PathBuf,
    events: PathBuf,
    out: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    use zutti_protocol::encoding::Writer;
    use zutti_wallet::{inventory::PoolInventory, private_note::EncryptedNote};
    if out.try_exists()? {
        return Err("inventory output already exists".into());
    }
    let profile = read_deployment(deployment)?;
    let (domain, _) = deployment_context(&profile)?;
    let domain = domain.id()?;
    let account = WalletAccount::open(&directory)?;
    let mut bytes = Vec::new();
    fs::File::open(events)?
        .take(16_777_217)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 16_777_216 {
        return Err("pool events exceed their bound".into());
    }
    let capture: serde_json::Value = serde_json::from_slice(&bytes)?;
    if capture["chain_id"] != 11155111
        || capture["pool"] != profile["pool"]
        || capture["domain"] != profile["domain"]
    {
        return Err("pool events deployment mismatch".into());
    }
    let root = Digest::from_hex(
        capture["root"]
            .as_str()
            .ok_or("missing root")?
            .trim_start_matches("0x"),
    )?;
    let size = u32::try_from(capture["next_leaf"].as_u64().ok_or("missing leaf count")?)?;
    let mut outputs = Vec::new();
    for event in capture["outputs"]
        .as_array()
        .ok_or("missing pool outputs")?
    {
        let position = u32::try_from(
            event["position"]
                .as_u64()
                .ok_or("missing output position")?,
        )?;
        let encoded = event["ciphertext"].as_str().ok_or("missing ciphertext")?;
        if encoded.len() > 8194 {
            return Err("pool output exceeds its bound".into());
        }
        let encrypted = EncryptedNote::from_bytes(&hex::decode(encoded.trim_start_matches("0x"))?)?;
        if encrypted.commitment().to_string()
            != event["commitment"]
                .as_str()
                .ok_or("missing commitment")?
                .trim_start_matches("0x")
        {
            return Err("pool output commitment mismatch".into());
        }
        outputs.push((position, encrypted));
    }
    let inventory = PoolInventory::scan(domain, account.note_keys(), &outputs, root, size)?;
    let mut writer = Writer::default();
    writer.fixed(b"ZUTTIINV");
    writer.u16(1);
    writer.fixed(&domain.0);
    writer.fixed(&root.0);
    writer.u32(size);
    writer.u32(u32::try_from(inventory.notes().len())?);
    let mut notes = Vec::new();
    for owned in inventory.notes() {
        owned.note.encode(&mut writer)?;
        owned.path.encode(&mut writer)?;
        let nullifier = account.note_keys().nullifier(&owned.note)?;
        notes.push(serde_json::json!({
            "position":owned.path.position,"commitment":owned.note.commitment()?.to_string(),
            "amount_units":owned.note.amount().units(),
            "request":{"chain_id":11155111,"pool":profile["pool"],"pool_code_hash":profile["code_hashes"]["pool"],"root":format!("0x{root}"),"nullifier":format!("0x{nullifier}")}
        }));
    }
    let plaintext = zeroize::Zeroizing::new(writer.finish());
    let raw_key = backup::read_private(&directory.join("account.key"), 32)?;
    let key = Secret32::from_bytes(raw_key.as_slice().try_into()?)?;
    let archive = backup::seal(domain, &plaintext, &key)?;
    let parent = out
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    let stage = tempfile::Builder::new()
        .prefix(".zutti-inventory-")
        .tempdir_in(parent)?;
    let result = serde_json::json!({"root":root.to_string(),"size":size,"notes":notes,"ethereum_verified":false});
    backup::persist_new(&stage.path().join("inventory.backup"), &archive)?;
    backup::persist_new(
        &stage.path().join("inventory.json"),
        &serde_json::to_vec_pretty(&result)?,
    )?;
    fs::rename(stage.path(), &out)?;
    fs::File::open(parent)?.sync_all()?;
    println!(
        "{}",
        serde_json::json!({"root":inventory.root().to_string(),"size":inventory.size(),"owned_notes":inventory.notes().len(),"ethereum_verified":false})
    );
    Ok(())
}

fn deposit(
    directory: PathBuf,
    deployment: PathBuf,
    amount: String,
    out: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    if out.try_exists()? {
        return Err("deposit output already exists".into());
    }
    let profile = read_deployment(deployment)?;
    let (domain, policy) = deployment_context(&profile)?;
    let address = |name: &str| -> Result<[u8; 20], Box<dyn std::error::Error>> {
        let value = profile[name].as_str().ok_or("missing deployment address")?;
        Ok(
            hex::decode(value.strip_prefix("0x").ok_or("invalid address")?)?
                .try_into()
                .map_err(|_| "invalid address length")?,
        )
    };
    let field = |name: &str| -> Result<&str, Box<dyn std::error::Error>> {
        Ok(profile[name].as_str().ok_or("missing deployment field")?)
    };
    let context = DepositContext::new(
        domain,
        policy,
        Digest::from_hex(field("binding_image")?)?,
        Digest::from_hex(field("pool_image")?)?,
        address("deployer")?,
    )?;
    let account = WalletAccount::open(&directory)?;
    let amount = Amount::parse(Asset::Usdc, &amount)?;
    let witness = context.prepare(account.note_keys(), amount)?;
    let plaintext = zeroize::Zeroizing::new(witness.to_bytes()?);
    let raw_key = backup::read_private(&directory.join("account.key"), 32)?;
    let key = Secret32::from_bytes(raw_key.as_slice().try_into()?)?;
    let archive = backup::seal(witness.claim.pool.domain, &plaintext, &key)?;
    let encrypted = witness.outputs[0].encrypted()?;
    let claim = &witness.claim.pool;
    let zero = format!("0x{}", "00".repeat(32));
    let result = serde_json::json!({
        "schema":1,"network":"testnet","pool":field("pool")?,
        "amount_units":claim.value,"proof_generated":false,"tokens_deposited":false,
        "journal":hex_bytes(&witness.claim.to_bytes()?),
        "operation":{
            "kind":1,"root":zero,"checkpoint":zero,"expiry":0,
            "orderTag":zero,"order":zero,"reservation":zero,
            "value":claim.value,"owner":format!("0x{}",hex_bytes(&claim.owner)),
            "nullifiers":[],"outputs":[{
                "commitment":format!("0x{}",encrypted.commitment()),
                "ciphertext":format!("0x{}",hex_bytes(&encrypted.to_bytes()?))
            }]
        }
    });
    let parent = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    let stage = tempfile::Builder::new()
        .prefix(".zutti-deposit-")
        .tempdir_in(parent)?;
    backup::persist_new(&stage.path().join("witness.bin"), &plaintext)?;
    backup::persist_new(&stage.path().join("deposit.backup"), &archive)?;
    backup::persist_new(
        &stage.path().join("deposit.json"),
        &serde_json::to_vec_pretty(&result)?,
    )?;
    fs::rename(stage.path(), &out)?;
    fs::File::open(parent)?.sync_all()?;
    println!(
        "{}",
        serde_json::json!({"prepared":true,"amount_units":claim.value,"proof_generated":false,"tokens_deposited":false})
    );
    Ok(())
}

fn deployment_context(
    profile: &serde_json::Value,
) -> Result<(Domain, Policy), Box<dyn std::error::Error>> {
    let field = |name: &str| -> Result<&str, Box<dyn std::error::Error>> {
        Ok(profile[name].as_str().ok_or("missing deployment field")?)
    };
    let address = |name: &str| -> Result<[u8; 20], Box<dyn std::error::Error>> {
        let value = field(name)?;
        Ok(
            hex::decode(value.strip_prefix("0x").ok_or("invalid address")?)?
                .try_into()
                .map_err(|_| "invalid address length")?,
        )
    };
    let policy = Policy::from_bytes(&hex::decode(field("source_policy")?)?)?;
    if !policy.paid_only()
        || policy.id()?.to_string() != field("source_policy_id")?
        || policy.program().to_string() != field("source_image")?
        || !field("token")?.eq_ignore_ascii_case("0x1c7D4B196Cb0C7B01d743Fbc6116a902379C7238")
    {
        return Err("unsupported source or token policy".into());
    }
    let domain = Domain::new(
        ZCASH_TESTNET_GENESIS,
        11155111,
        address("pool")?,
        address("token")?,
        zutti_wallet::binding::payment_profile(),
        policy.id()?,
        Digest::from_hex(field("proof_profile_id")?)?,
    )?;
    if domain.id()?.to_string() != field("domain")? {
        return Err("deployment domain mismatch".into());
    }
    Ok((domain, policy))
}

fn read_deployment(path: PathBuf) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let mut data = Vec::new();
    fs::File::open(path)?.take(65_537).read_to_end(&mut data)?;
    if data.len() > 65_536 {
        return Err("deployment exceeds its bound".into());
    }
    let profile: serde_json::Value = serde_json::from_slice(&data)?;
    if profile["network"] != "testnet" || profile["chain_id"] != 11155111 {
        return Err("Testnet deployment required".into());
    }
    Ok(profile)
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
