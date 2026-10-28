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
    ShareOrder {
        #[arg(long)]
        directory: PathBuf,
        #[arg(long)]
        deployment: PathBuf,
        #[arg(long)]
        order: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    Order {
        #[arg(long)]
        directory: PathBuf,
        #[arg(long)]
        deployment: PathBuf,
        #[arg(long)]
        notes: PathBuf,
        #[arg(long)]
        anchor: PathBuf,
        #[arg(long)]
        offer: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
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
        block: Vec<PathBuf>,
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
        Command::ShareOrder {
            directory,
            deployment,
            order,
            out,
        } => {
            let profile = read_deployment(deployment)?;
            let (domain, _) = deployment_context(&profile)?;
            WalletAccount::open(&directory)?;
            let key = backup::read_private(&directory.join("account.key"), 32)?;
            let key = Secret32::from_bytes(key.as_slice().try_into()?)?;
            let prepared = zutti_wallet::order::PreparedOrder::from_bytes(&backup::open(
                domain.id()?,
                &backup::read_private(&order, 262_144)?,
                &key,
            )?)?;
            if prepared.reservation().terms.domain() != &domain {
                return Err("order deployment mismatch".into());
            }
            backup::persist_new(&out, &prepared.reservation().to_bytes()?)?;
            println!(
                "{}",
                serde_json::json!({"reservation_shared":true,"authorized":false})
            );
            return Ok(());
        }
        Command::Order {
            directory,
            deployment,
            notes,
            anchor,
            offer,
            out,
        } => return order(directory, deployment, notes, anchor, offer, out),
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

fn canonical_json(value: &serde_json::Value) -> Result<String, Box<dyn std::error::Error>> {
    use serde_json::Value;
    Ok(match value {
        Value::Object(fields) => {
            let mut keys: Vec<_> = fields.keys().collect();
            keys.sort();
            let fields = keys
                .into_iter()
                .map(|key| {
                    Ok(format!(
                        "{}:{}",
                        serde_json::to_string(key)?,
                        canonical_json(&fields[key])?
                    ))
                })
                .collect::<Result<Vec<_>, Box<dyn std::error::Error>>>()?;
            format!("{{{}}}", fields.join(","))
        }
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonical_json)
                .collect::<Result<Vec<_>, _>>()?
                .join(",")
        ),
        Value::Number(number) => {
            let value = number.as_i64().ok_or("RFQ numbers must be safe integers")?;
            if value.unsigned_abs() > 9_007_199_254_740_991 {
                return Err("RFQ number exceeds its bound".into());
            }
            value.to_string()
        }
        _ => serde_json::to_string(value)?,
    })
}

fn order(
    directory: PathBuf,
    deployment: PathBuf,
    notes: PathBuf,
    anchor: PathBuf,
    offer: PathBuf,
    out: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    use sha2::Digest as _;
    use zutti_protocol::{amount::FeeBps, private_note::NoteRecipient};
    use zutti_wallet::{
        order::{MakerRecipients, OrderContext, PreparedOrder, QuoteTerms},
        source_notes::SourceScan,
    };
    if out.try_exists()? {
        return Err("order output already exists".into());
    }
    let profile = read_deployment(deployment)?;
    let (domain, policy) = deployment_context(&profile)?;
    let account = WalletAccount::open(&directory)?;
    let key = backup::read_private(&directory.join("account.key"), 32)?;
    let key = Secret32::from_bytes(key.as_slice().try_into()?)?;
    let scan = SourceScan::from_bytes(&backup::open(
        ZCASH_TESTNET_GENESIS,
        &backup::read_private(&notes, 262_144)?,
        &key,
    )?)?;
    let mut anchor_bytes = Vec::new();
    fs::File::open(anchor)?
        .take(16_385)
        .read_to_end(&mut anchor_bytes)?;
    if anchor_bytes.len() > 16_384 {
        return Err("source anchor exceeds its bound".into());
    }
    let anchor = zutti_source::spv::HeaderClaim::from_bytes(&anchor_bytes)?;
    let mut offer: serde_json::Value =
        serde_json::from_slice(&backup::read_private(&offer, 65_536)?)?;
    let id = Digest::from_hex(
        offer
            .as_object_mut()
            .ok_or("invalid offer")?
            .remove("id")
            .and_then(|value| value.as_str().map(str::to_owned))
            .ok_or("missing offer ID")?
            .as_str(),
    )?;
    let hash=sha2::Sha256::digest(canonical_json(&serde_json::json!({"protocol":"zutti-rfq","version":1,"purpose":"offer","value":offer}))?.as_bytes());
    if Digest(hash.into()) != id {
        return Err("offer commitment mismatch".into());
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis();
    if offer["deadline"]
        .as_u64()
        .is_none_or(|deadline| u128::from(deadline) <= now)
        || offer["input"]["asset"] != "ZEC"
        || offer["gross_output"]["asset"] != "USDC"
        || offer["output"]["asset"] != "USDC"
        || offer["maker_fee"]["amount"]["asset"] != "USDC"
        || offer["policy"]["source_policy_id"] != domain.source_policy().to_string()
        || offer["policy"]["payment_profile_id"] != domain.payment_profile().to_string()
        || offer["policy"]["proof_profile_id"] != domain.proof_profile().to_string()
        || offer["policy"]["ethereum_finality"] != "finalized"
    {
        return Err("offer terms or deployment mismatch".into());
    }
    let text = |value: &serde_json::Value| -> Result<String, Box<dyn std::error::Error>> {
        Ok(value.as_str().ok_or("missing offer field")?.to_owned())
    };
    let zec = Amount::parse(Asset::Zec, &text(&offer["input"]["amount"])?)?;
    let gross = Amount::parse(Asset::Usdc, &text(&offer["gross_output"]["amount"])?)?;
    let fee = FeeBps::new(u16::try_from(
        offer["maker_fee"]["bps"]
            .as_u64()
            .ok_or("missing maker fee")?,
    )?)?;
    let net = gross.net_after_fee(fee);
    if net != Amount::parse(Asset::Usdc, &text(&offer["output"]["amount"])?)?
        || gross.checked_sub(net)?
            != Amount::parse(Asset::Usdc, &text(&offer["maker_fee"]["amount"]["amount"])?)?
    {
        return Err("offer fee arithmetic mismatch".into());
    }
    let mut caps = [
        Amount::from_units(Asset::Zec, 0)?,
        Amount::from_units(Asset::Eth, 0)?,
        Amount::from_units(Asset::Usdc, 0)?,
    ];
    let mut seen = std::collections::BTreeSet::new();
    for item in offer["fees"].as_array().ok_or("missing fee caps")? {
        let kind = text(&item["kind"])?;
        let (index, asset) = match kind.as_str() {
            "source_network" => (0, Asset::Zec),
            "ethereum_execution" => (1, Asset::Eth),
            "submission" => (2, Asset::Usdc),
            _ => return Err("unsupported fee kind".into()),
        };
        if !seen.insert(index) || item["asset"] != ["ZEC", "ETH", "USDC"][index] {
            return Err("invalid or duplicate fee cap".into());
        }
        caps[index] = Amount::parse(asset, &text(&item["max_amount"])?)?;
    }
    if !seen.contains(&0) || !seen.contains(&1) {
        return Err("missing network fee caps".into());
    }
    let descriptor = &offer["receiving"];
    let refund = NoteRecipient::new(
        hex::decode(text(&descriptor["usdc"]["owner"])?)?
            .try_into()
            .map_err(|_| "invalid note owner")?,
        hex::decode(text(&descriptor["usdc"]["viewing"])?)?
            .try_into()
            .map_err(|_| "invalid note viewing key")?,
        Digest::from_hex(&text(&descriptor["usdc"]["nullifier_key_commitment"])?)?,
    )?;
    let maker = MakerRecipients::from_address(&text(&descriptor["zcash"])?, refund)?;
    let prepared = PreparedOrder::prepare(
        OrderContext {
            domain: domain.clone(),
            policy,
            anchor,
            source: &scan,
            viewing: &account.source_viewing_key(),
            payout: account.private_receiver()?,
        },
        QuoteTerms {
            offer: id,
            zec,
            gross_usdc: gross,
            fee_bps: fee,
            fee_caps: caps,
            nonce: Digest::from_hex(&text(&offer["nonce"])?)?,
        },
        maker,
    )?;
    let parent = out
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    let stage = tempfile::Builder::new()
        .prefix(".zutti-order-")
        .tempdir_in(parent)?;
    let reservation = prepared.reservation();
    let plaintext = zeroize::Zeroizing::new(prepared.to_bytes()?);
    backup::persist_new(
        &stage.path().join("order.backup"),
        &backup::seal(domain.id()?, &plaintext, &key)?,
    )?;
    backup::persist_new(
        &stage.path().join("binding-witness.bin"),
        &prepared.binding_witness()?.to_bytes()?,
    )?;
    backup::persist_new(&stage.path().join("unsigned.pczt"), prepared.payment())?;
    backup::persist_new(
        &stage.path().join("terms.bin"),
        &reservation.terms.to_bytes()?,
    )?;
    backup::persist_new(
        &stage.path().join("randomness.key"),
        reservation.order_randomness.expose(),
    )?;
    let summary = serde_json::json!({"offer":id.to_string(),"txid":reservation.txid.to_string(),"source_height":scan.height,"expiry":reservation.terms.window().expiry(),"zec":zec.to_string(),"usdc":net.to_string(),"authorized":false,"broadcast":false});
    backup::persist_new(
        &stage.path().join("order.json"),
        &serde_json::to_vec_pretty(&summary)?,
    )?;
    fs::rename(stage.path(), &out)?;
    fs::File::open(parent)?.sync_all()?;
    println!("{summary}");
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
    block: Vec<PathBuf>,
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
    if block.is_empty()
        || block.len() > 4096
        || current["height"].as_u64() != Some(u64::from(height) + block.len() as u64)
    {
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
    let blocks = block
        .into_iter()
        .map(|file| -> zutti_wallet::Result<Vec<u8>> {
            let mut block_data = Vec::new();
            fs::File::open(file)?
                .take(2_000_001)
                .read_to_end(&mut block_data)?;
            Ok(block_data)
        });
    let scan = zutti_wallet::source_notes::scan_ironwood_stream(
        &account.source_viewing_key(),
        height,
        parent,
        &tree,
        blocks,
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
    use zutti_wallet::inventory::{PoolInventory, PoolOutput};
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
        let encrypted = PoolOutput::from_bytes(&hex::decode(encoded.trim_start_matches("0x"))?)?;
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
    let inventory = PoolInventory::scan_outputs(domain, account.note_keys(), &outputs, root, size)?;
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
