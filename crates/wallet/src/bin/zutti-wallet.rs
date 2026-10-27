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
