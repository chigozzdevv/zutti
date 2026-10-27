use alloy::primitives::{Address, B256};
use bincode::Options;
use clap::{Args as ClapArgs, Subcommand};
use risc0_zkvm::{Receipt, VerifierContext};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command as Process, Stdio},
};
use zutti_ethereum::{
    ConsensusConfig, Evidence,
    inventory::InventoryEvidence,
    notes::NoteEvidence,
    spv_authorization::{SpvAuthorizationProfile, SpvPaymentConsent, SpvSourceEvidence},
    spv_funding::{FundingEvidence, NoteFundingEvidence, SpvFundingProfile},
};
use zutti_protocol::{
    Digest, Secret32,
    amount::{Amount, Asset},
    domain::Domain,
    encoding::Canonical,
    pool::{OperationKind, PoolClaim, nullifier_digest, output_digest},
};
use zutti_source::spv::{HEADER_BYTES, HeaderClaim, Policy};
use zutti_wallet::{
    account::WalletAccount,
    backup,
    funding::{FundingConsent, SpvFundingPlan},
    inventory::PoolInventory,
    order::PreparedOrder,
    payment::{PaymentState, SpvPaymentJournal},
    pool::{NoteInput, NoteOutput},
    private_note::NoteEncryption,
    spv_pool::{SpvAction, SpvPoolClaim, SpvPoolWitness, SpvReservation, recovery::SpvSwapRecord},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Debug, ClapArgs)]
pub struct Args {
    #[arg(long)]
    deployment: PathBuf,
    #[arg(long)]
    config: PathBuf,
    #[arg(long)]
    directory: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    FundingCheck {
        #[arg(long)]
        witness: PathBuf,
        #[arg(long)]
        funding: PathBuf,
        #[arg(long)]
        source: PathBuf,
        #[arg(long)]
        headers: PathBuf,
    },
    FundingAuthorizeNote {
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        binding: PathBuf,
        #[arg(long)]
        anchor: PathBuf,
        #[arg(long)]
        evidence: PathBuf,
        #[arg(long)]
        headers: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    PoolExport {
        #[arg(long)]
        witness: PathBuf,
        #[arg(long)]
        proof: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    WindowPrepare {
        #[arg(long, required_unless_present = "record", conflicts_with = "record")]
        funding: Option<PathBuf>,
        #[arg(long, required_unless_present = "funding", conflicts_with = "funding")]
        record: Option<PathBuf>,
        #[arg(long)]
        headers: PathBuf,
        #[arg(long)]
        block: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    SettlementPrepare {
        #[arg(long, required_unless_present = "record", conflicts_with = "record")]
        funding: Option<PathBuf>,
        #[arg(long, required_unless_present = "funding", conflicts_with = "funding")]
        record: Option<PathBuf>,
        #[arg(long)]
        window: PathBuf,
        #[arg(long)]
        events: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    FundingPrepare {
        #[arg(long)]
        inventory: PathBuf,
        #[arg(long)]
        reservation: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    FundingAuthorize {
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        binding: PathBuf,
        #[arg(long)]
        anchor: PathBuf,
        #[arg(long)]
        evidence: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    PaymentPrepare {
        #[arg(long)]
        witness: PathBuf,
        #[arg(long)]
        funding: PathBuf,
        #[arg(long)]
        pczt: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    PaymentSend {
        #[arg(long)]
        journal: PathBuf,
        #[arg(long)]
        swap: String,
        #[arg(long)]
        order: PathBuf,
        #[arg(long)]
        evidence: PathBuf,
        #[arg(long)]
        source: PathBuf,
        #[arg(long)]
        headers: Option<PathBuf>,
        #[arg(long)]
        continuation: PathBuf,
        #[arg(long)]
        transport: PathBuf,
    },
    Withdraw {
        #[arg(long)]
        inventory: PathBuf,
        #[arg(long)]
        position: u32,
        #[arg(long)]
        amount: String,
        #[arg(long)]
        owner: Address,
        #[arg(long)]
        evidence: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
}

struct Context {
    profile: SpvAuthorizationProfile,
    registry: Address,
    registry_code: B256,
    account: WalletAccount,
    key: Secret32,
}

fn read<T: serde::de::DeserializeOwned>(path: &Path, limit: usize) -> Result<T> {
    Ok(serde_json::from_slice(&bounded(path, limit)?)?)
}

fn bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    if !std::fs::symlink_metadata(path)?.is_file() {
        return Err("regular input file required".into());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err("input exceeds its bound".into());
    }
    Ok(bytes)
}

fn receipt(path: &Path) -> Result<Receipt> {
    Ok(bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(16_777_216)
        .reject_trailing_bytes()
        .deserialize(&bounded(path, 16_777_216)?)?)
}

fn verify(receipt: &Receipt, image: Digest, journal: &[u8]) -> Result<()> {
    receipt.verify_with_context(&VerifierContext::default().with_dev_mode(false), image.0)?;
    if receipt.journal.bytes != journal {
        return Err("receipt journal mismatch".into());
    }
    Ok(())
}

impl Context {
    fn open(args: &Args) -> Result<Self> {
        let manifest: serde_json::Value = read(&args.deployment, 65_536)?;
        if manifest["network"] != "testnet" || manifest["chain_id"] != 11155111 {
            return Err("Sepolia Testnet deployment required".into());
        }
        let text = |name: &str| -> Result<&str> {
            Ok(manifest[name].as_str().ok_or("missing deployment field")?)
        };
        let domain = Domain::from_bytes(&alloy::hex::decode(text("domain_encoding")?)?)?;
        let policy = Policy::from_bytes(&alloy::hex::decode(text("source_policy")?)?)?;
        let consensus: ConsensusConfig = read(&args.config, 65_536)?;
        if !policy.paid_only()
            || domain.id()?.to_string() != text("domain")?
            || domain.source_genesis() != zutti_protocol::network::ZCASH_TESTNET_GENESIS
            || domain.destination_chain() != 11155111
            || consensus.chain_id != 11155111
            || domain.source_policy() != policy.id()?
            || policy.id()?.to_string() != text("source_policy_id")?
            || policy.program().to_string() != text("source_image")?
            || Address::from(domain.pool()) != text("pool")?.parse::<Address>()?
            || Address::from(domain.token()) != text("token")?.parse::<Address>()?
        {
            return Err("deployment profile mismatch".into());
        }
        let account = WalletAccount::open(&args.directory)?;
        let key = backup::read_private(&args.directory.join("account.key"), 32)?;
        let key = Secret32::from_bytes(key.as_slice().try_into()?)?;
        Ok(Self {
            profile: SpvAuthorizationProfile {
                domain,
                ethereum: consensus,
                pool_code_hash: manifest["code_hashes"]["pool"]
                    .as_str()
                    .ok_or("missing pool code hash")?
                    .parse()?,
                source: text("registry")?.parse()?,
                source_code_hash: manifest["code_hashes"]["registry"]
                    .as_str()
                    .ok_or("missing registry code hash")?
                    .parse()?,
                binding_program: Digest::from_hex(text("binding_image")?)?,
                pool_program: Digest::from_hex(text("pool_image")?)?,
                policy,
                minimum_blocks: 64,
                maximum_source_age: 3600,
                future_drift: 300,
            },
            registry: text("registry")?.parse()?,
            registry_code: manifest["code_hashes"]["registry"]
                .as_str()
                .ok_or("missing registry code hash")?
                .parse()?,
            account,
            key,
        })
    }
    fn recovery_record(
        &self,
        funding: Option<PathBuf>,
        record: Option<PathBuf>,
    ) -> Result<SpvSwapRecord> {
        let record = match (funding, record) {
            (Some(funding), None) => SpvSwapRecord::from_reservation(&SpvPoolWitness::from_bytes(
                &backup::read_private(&funding, 262_144)?,
            )?)?,
            (None, Some(record)) => SpvSwapRecord::from_bytes(&backup::open(
                self.profile.domain.id()?,
                &backup::read_private(&record, 263_168)?,
                &self.key,
            )?)?,
            _ => return Err("exactly one recovery source required".into()),
        };
        self.profile.reservation_request(&record)?;
        Ok(record)
    }
    fn inventory(&self, path: &Path) -> Result<PoolInventory> {
        Ok(PoolInventory::from_bytes(&backup::open(
            self.profile.domain.id()?,
            &backup::read_private(path, 262_144)?,
            &self.key,
        )?)?)
    }
    fn plan(&self, path: &Path) -> Result<SpvFundingPlan> {
        Ok(SpvFundingPlan::from_bytes(&backup::open(
            self.profile.domain.id()?,
            &backup::read_private(path, 262_144)?,
            &self.key,
        )?)?)
    }
    fn funding(&self) -> SpvFundingProfile<'_> {
        SpvFundingProfile {
            swap: &self.profile,
            registry: self.registry,
            registry_code_hash: self.registry_code,
        }
    }
    fn seal(&self, bytes: &[u8]) -> Result<Vec<u8>> {
        Ok(backup::seal(self.profile.domain.id()?, bytes, &self.key)?)
    }
    fn consent<'a>(
        &self,
        terms: &'a zutti_protocol::order::OrderTerms,
    ) -> Result<FundingConsent<'a>> {
        if terms.receiver() != &self.account.source_receiver() {
            return Err("maker Zcash recipient mismatch".into());
        }
        Ok(FundingConsent {
            terms,
            refund: self.account.private_receiver()?,
            policy: self.profile.policy,
            binding_program: self.profile.binding_program,
            pool_program: self.profile.pool_program,
        })
    }
}

fn stage(out: &Path, files: &[(&str, Vec<u8>)]) -> Result<()> {
    if out.try_exists()? {
        return Err("operation output already exists".into());
    }
    let parent = out.parent().ok_or("missing output parent")?;
    let temporary = parent.join(format!(".zutti-stage-{}", std::process::id()));
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&temporary)?;
    for (name, bytes) in files {
        backup::persist_new(&temporary.join(name), bytes)?;
    }
    std::fs::rename(&temporary, out)?;
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

pub fn run(args: Args, now: u64) -> Result<()> {
    let context = Context::open(&args)?;
    match args.command {
        Command::FundingCheck {
            witness,
            funding,
            source,
            headers,
        } => {
            let witness = SpvPoolWitness::from_bytes(&backup::read_private(&witness, 262_144)?)?;
            let record = SpvSwapRecord::from_reservation(&witness)?;
            context.profile.reservation_request(&record)?;
            context.consent(record.terms())?;
            verify(
                &receipt(&funding)?,
                context.profile.pool_program,
                &witness.evaluate()?.to_bytes()?,
            )?;
            let source = receipt(&source)?;
            let anchor = &record.reservation().anchor;
            verify(
                &source,
                context.profile.policy.program(),
                &anchor.to_bytes()?,
            )?;
            let bytes = bounded(&headers, 512 * HEADER_BYTES)?;
            if bytes.len() % HEADER_BYTES != 0 {
                return Err("invalid current funding headers".into());
            }
            let headers = bytes
                .chunks_exact(HEADER_BYTES)
                .map(<[u8]>::to_vec)
                .collect::<Vec<_>>();
            let current = anchor.advance(&headers)?;
            let height = current.height()?;
            record
                .terms()
                .window()
                .require_remaining(height, context.profile.minimum_blocks)?;
            let timestamp = u64::from(current.timestamp()?);
            if timestamp > now.saturating_add(context.profile.future_drift)
                || u64::from(current.maximum_time())
                    > now.saturating_add(context.profile.future_drift)
                || now > timestamp.saturating_add(context.profile.maximum_source_age)
            {
                return Err("stale current funding headers".into());
            }
            println!(
                "{}",
                serde_json::json!({"funding_proof_verified":true,"current_headers_verified":true,"height":height,"remaining_blocks":record.terms().window().expiry()-height,"funded":false})
            );
        }
        Command::FundingAuthorizeNote {
            plan,
            binding,
            anchor,
            evidence,
            headers,
            out,
        } => {
            let plan = context.plan(&plan)?;
            let terms = plan.reservation().terms.clone();
            let consent = context.consent(&terms)?;
            let headers = bounded(&headers, 512 * HEADER_BYTES)?;
            if headers.len() % HEADER_BYTES != 0 {
                return Err("invalid current source headers".into());
            }
            let headers: Vec<_> = headers
                .chunks_exact(HEADER_BYTES)
                .map(<[u8]>::to_vec)
                .collect();
            let witness = context.funding().authorize_from_note(
                plan,
                &consent,
                context.account.note_keys(),
                NoteFundingEvidence {
                    ethereum: &read::<NoteEvidence>(&evidence, 16_777_216)?,
                    binding: &receipt(&binding)?,
                    anchor: &receipt(&anchor)?,
                    headers: &headers,
                },
                now,
            )?;
            let record = SpvSwapRecord::from_reservation(&witness)?;
            stage(
                &out,
                &[
                    ("witness.bin", witness.to_bytes()?),
                    ("journal.bin", witness.claim.to_bytes()?),
                    ("swap.backup", context.seal(&record.to_bytes()?)?),
                ],
            )?;
            println!(
                "{}",
                serde_json::json!({"funding_authorized":true,"ethereum_note_finality_verified":true,"current_headers_validated_locally":true,"funded":false,"swap":record.id()?.to_string()})
            );
        }
        Command::PoolExport {
            witness,
            proof,
            out,
        } => {
            let witness = SpvPoolWitness::from_bytes(&backup::read_private(&witness, 262_144)?)?;
            let evaluated = witness.evaluate()?;
            if evaluated.policy != context.profile.policy.id()?
                || evaluated.binding_program != context.profile.binding_program
                || evaluated.pool_program != context.profile.pool_program
                || evaluated.pool.domain != context.profile.domain.id()?
            {
                return Err("pool operation profile mismatch".into());
            }
            let proof = receipt(&proof)?;
            verify(&proof, context.profile.pool_program, &evaluated.to_bytes()?)?;
            let risc0_zkvm::InnerReceipt::Groth16(inner) = &proof.inner else {
                return Err("Groth16 pool proof required".into());
            };
            use risc0_zkvm::sha::Digestible;
            let parameters = risc0_zkvm::Groth16ReceiptVerifierParameters::default().digest();
            if inner.verifier_parameters != parameters || inner.seal.len() != 256 {
                return Err("unsupported pool verifier".into());
            }
            let mut seal = parameters.as_bytes()[..4].to_vec();
            seal.extend_from_slice(&inner.seal);
            let mut nullifiers = witness
                .inputs
                .iter()
                .map(|input| input.note.nullifier(&input.nullifier_key))
                .collect::<zutti_protocol::Result<Vec<_>>>()?;
            let mut outputs = Vec::new();
            match &witness.action {
                SpvAction::Reserve(reservation) => outputs.push((
                    reservation.escrow.commitment()?,
                    reservation.escrow.encrypt(reservation.nonce)?,
                )),
                SpvAction::Settle { reservation, .. } => {
                    nullifiers.push(reservation.escrow.nullifier()?)
                }
                _ => {}
            }
            for output in &witness.outputs {
                outputs.push((output.note.commitment()?, output.encrypted()?.to_bytes()?));
            }
            let prefix = |digest: Digest| format!("0x{digest}");
            let claim = &evaluated.pool;
            let value = serde_json::json!({"chain_id":11155111,"pool":Address::from(context.profile.domain.pool()),"proof_verified":true,
                "seal":format!("0x{}",alloy::hex::encode(seal)),"journal":alloy::hex::encode(evaluated.to_bytes()?),
                "operation":{"kind":claim.kind as u8,"root":prefix(claim.root),"checkpoint":prefix(claim.checkpoint),"expiry":evaluated.expiry,
                    "orderTag":prefix(claim.order_tag),"order":prefix(claim.order),"reservation":prefix(claim.reservation),"value":claim.value,
                    "owner":Address::from(claim.owner),"nullifiers":nullifiers.into_iter().map(prefix).collect::<Vec<_>>(),
                    "outputs":outputs.into_iter().map(|(commitment,ciphertext)|serde_json::json!({"commitment":prefix(commitment),"ciphertext":format!("0x{}",alloy::hex::encode(ciphertext))})).collect::<Vec<_>>()}});
            backup::persist_new(&out, &serde_json::to_vec_pretty(&value)?)?;
            println!(
                "{}",
                serde_json::json!({"pool_proof_verified":true,"kind":claim.kind as u8,"transaction_submitted":false})
            );
        }
        Command::WindowPrepare {
            funding,
            record,
            headers,
            block,
            out,
        } => {
            use zutti_source::spv::{Branch, WindowWitness, Witness};
            let record = context.recovery_record(funding, record)?;
            let reservation = record.reservation();
            let headers = bounded(&headers, 512 * HEADER_BYTES)?;
            if headers.is_empty() || headers.len() % HEADER_BYTES != 0 {
                return Err("invalid payment headers".into());
            }
            let headers: Vec<_> = headers
                .chunks_exact(HEADER_BYTES)
                .map(<[u8]>::to_vec)
                .collect();
            let end = reservation.anchor.advance(&headers)?;
            let window = WindowWitness::new(
                reservation.window()?,
                reservation.txid,
                Secret32::from_bytes(*reservation.reservation_randomness.expose())?,
                Branch::Included,
                headers,
                vec![bounded(&block, zutti_source::block::MAX_BLOCK_BYTES)?],
            )?;
            let witness = Witness::Window {
                previous: reservation.anchor.clone(),
                window: Box::new(window),
            };
            let claim = witness.evaluate()?.journal()?;
            stage(
                &out,
                &[
                    ("witness.bin", witness.to_bytes()?),
                    ("journal.bin", claim),
                    ("end-header.bin", end.to_bytes()?),
                ],
            )?;
            println!(
                "{}",
                serde_json::json!({"payment_inclusion_checked":true,"height":end.height()?,"proof_generated":false})
            );
        }
        Command::SettlementPrepare {
            funding,
            record,
            window,
            events,
            out,
        } => {
            let record = context.recovery_record(funding, record)?;
            let window = zutti_source::spv::WindowClaim::from_bytes(&bounded(&window, 4096)?)?;
            let events: serde_json::Value = read(&events, 16_777_216)?;
            if events["chain_id"] != 11155111
                || events["pool"]
                    .as_str()
                    .ok_or("missing event pool")?
                    .parse::<Address>()?
                    != Address::from(context.profile.domain.pool())
                || events["domain"]
                    .as_str()
                    .ok_or("missing event domain")?
                    .trim_start_matches("0x")
                    != context.profile.domain.id()?.to_string()
            {
                return Err("pool event domain mismatch".into());
            }
            let mut tree = zutti_protocol::pool::PoolTree::new(context.profile.domain.id()?)?;
            let escrow = record.reservation().escrow.commitment()?;
            let mut position = None;
            let mut seen = std::collections::BTreeSet::new();
            for (index, item) in events["outputs"]
                .as_array()
                .ok_or("missing output events")?
                .iter()
                .enumerate()
            {
                if index >= 65_536 || item["position"].as_u64() != Some(index as u64) {
                    return Err("non-contiguous pool events".into());
                }
                let commitment = Digest::from_hex(
                    item["commitment"]
                        .as_str()
                        .ok_or("missing output commitment")?
                        .trim_start_matches("0x"),
                )?;
                if !seen.insert(commitment) {
                    return Err("duplicate output commitment".into());
                }
                if commitment == escrow {
                    position = Some(u32::try_from(index)?);
                }
                tree.append(commitment)?;
            }
            if events["next_leaf"].as_u64() != Some(tree.size())
                || tree.root().to_string()
                    != events["root"]
                        .as_str()
                        .ok_or("missing pool root")?
                        .trim_start_matches("0x")
            {
                return Err("pool tree root mismatch".into());
            }
            let witness = record.settlement(
                tree.root(),
                tree.prove(position.ok_or("reserved escrow not found")?)?,
                window,
            )?;
            stage(
                &out,
                &[
                    ("witness.bin", witness.to_bytes()?),
                    ("journal.bin", witness.claim.to_bytes()?),
                ],
            )?;
            println!(
                "{}",
                serde_json::json!({"payout_prepared":true,"paid_out":false})
            );
        }
        Command::FundingPrepare {
            inventory,
            reservation,
            out,
        } => {
            let reservation =
                SpvReservation::from_bytes(&backup::read_private(&reservation, 262_144)?)?;
            context.consent(&reservation.terms)?;
            let plan = SpvFundingPlan::from_inventory(
                context.profile.policy,
                context.profile.binding_program,
                context.profile.pool_program,
                &context.inventory(&inventory)?,
                context.account.note_keys(),
                Box::new(reservation),
            )?;
            let request = context
                .funding()
                .request(&plan, context.account.note_keys())?;
            let nullifiers = plan.nullifiers(context.account.note_keys())?;
            if nullifiers.len() != 1 {
                return Err("single-note funding required".into());
            }
            let note_request = zutti_ethereum::notes::NoteRequest {
                chain_id: request.chain_id,
                pool: request.pool,
                pool_code_hash: request.pool_code_hash,
                root: request.root,
                nullifier: B256::from(nullifiers[0].0),
            };
            let info = serde_json::json!({"authorized":false,"funded":false,"expiry":plan.claim().expiry,"order":plan.claim().pool.order.to_string()});
            stage(
                &out,
                &[
                    ("plan.backup", context.seal(&plan.to_bytes()?)?),
                    ("request.json", serde_json::to_vec_pretty(&request)?),
                    (
                        "note-request.json",
                        serde_json::to_vec_pretty(&note_request)?,
                    ),
                    ("plan.json", serde_json::to_vec_pretty(&info)?),
                ],
            )?;
            println!("{info}");
        }
        Command::FundingAuthorize {
            plan,
            binding,
            anchor,
            evidence,
            out,
        } => {
            let plan = context.plan(&plan)?;
            let terms = plan.reservation().terms.clone();
            let consent = context.consent(&terms)?;
            let witness = context.funding().authorize(
                plan,
                &consent,
                context.account.note_keys(),
                FundingEvidence {
                    ethereum: &read::<InventoryEvidence>(&evidence, 16_777_216)?,
                    binding: &receipt(&binding)?,
                    anchor: &receipt(&anchor)?,
                },
                now,
            )?;
            let record = SpvSwapRecord::from_reservation(&witness)?;
            stage(
                &out,
                &[
                    ("witness.bin", witness.to_bytes()?),
                    ("journal.bin", witness.claim.to_bytes()?),
                    ("swap.backup", context.seal(&record.to_bytes()?)?),
                ],
            )?;
            println!(
                "{}",
                serde_json::json!({"funding_authorized":true,"ethereum_finality_verified":true,"funded":false,"swap":record.id()?.to_string()})
            );
        }
        Command::PaymentPrepare {
            witness,
            funding,
            pczt,
            out,
        } => {
            let witness = SpvPoolWitness::from_bytes(&backup::read_private(&witness, 262_144)?)?;
            let funding = receipt(&funding)?;
            verify(
                &funding,
                context.profile.pool_program,
                &witness.evaluate()?.to_bytes()?,
            )?;
            if witness.claim.pool.domain != context.profile.domain.id()? {
                return Err("funding domain mismatch".into());
            }
            let risc0_zkvm::InnerReceipt::Groth16(inner) = &funding.inner else {
                return Err("Groth16 funding proof required".into());
            };
            use risc0_zkvm::sha::Digestible;
            let parameters = risc0_zkvm::Groth16ReceiptVerifierParameters::default().digest();
            if inner.verifier_parameters != parameters {
                return Err("unsupported funding verifier".into());
            }
            let mut seal = parameters.as_bytes()[..4].to_vec();
            seal.extend_from_slice(&inner.seal);
            let record = SpvSwapRecord::from_reservation(&witness)?.with_funding_seal(seal)?;
            if record.reservation().payout.note.recipient() != context.account.private_receiver()? {
                return Err("taker payout ownership mismatch".into());
            }
            let request = context.profile.reservation_request(&record)?;
            let id = record.id()?;
            let key = Secret32::from_bytes(*context.key.expose())?;
            let journal = SpvPaymentJournal::create(
                &out,
                key,
                record,
                backup::read_private(&pczt, zutti_wallet::prepare::MAX_PCZT)?.to_vec(),
            )?;
            journal
                .record()
                .save_new(&out.join("swap.backup"), &context.key)?;
            backup::persist_new(
                &out.join("request.json"),
                &serde_json::to_vec_pretty(&request)?,
            )?;
            println!(
                "{}",
                serde_json::json!({"swap":id.to_string(),"state":format!("{:?}",journal.state()?),"zec_authorized":false})
            );
        }
        Command::PaymentSend {
            journal,
            swap,
            order,
            evidence,
            source,
            headers,
            continuation,
            transport,
        } => {
            let mut profile = context.profile.clone();
            profile.minimum_blocks = 16;
            let key = Secret32::from_bytes(*context.key.expose())?;
            let journal = SpvPaymentJournal::open(
                &journal,
                key,
                context.profile.domain.id()?,
                Digest::from_hex(&swap)?,
            )?;
            let prepared = PreparedOrder::from_bytes(&backup::open(
                context.profile.domain.id()?,
                &backup::read_private(&order, 262_144)?,
                &context.key,
            )?)?;
            let source = receipt(&source)?;
            let head = HeaderClaim::from_bytes(&source.journal.bytes)?;
            let headers = headers
                .map(|path| bounded(&path, 512 * HEADER_BYTES))
                .transpose()?
                .unwrap_or_default();
            if headers.len() % HEADER_BYTES != 0 {
                return Err("invalid source continuation".into());
            }
            let headers: Vec<_> = headers
                .chunks_exact(HEADER_BYTES)
                .map(<[u8]>::to_vec)
                .collect();
            let continuation = bounded(&continuation, 512 * HEADER_BYTES)?;
            if continuation.len() % HEADER_BYTES != 0 {
                return Err("invalid current source continuation".into());
            }
            let continuation: Vec<_> = continuation
                .chunks_exact(HEADER_BYTES)
                .map(<[u8]>::to_vec)
                .collect();
            let authorized = profile.verify(
                &journal,
                SpvPaymentConsent {
                    terms: &prepared.reservation().terms,
                    receiving: context.account.private_receiver()?,
                },
                &read::<Evidence>(&evidence, 16_777_216)?,
                SpvSourceEvidence {
                    receipt: &source,
                    head: &head,
                    headers_from_anchor: &headers,
                    continuation: &continuation,
                },
                now,
            )?;
            if journal.state()? == PaymentState::Prepared {
                authorized.sign(now, |pczt| context.account.sign_payment(pczt))?;
            }
            if journal.state()? == PaymentState::Signed {
                authorized.finalize()?;
            }
            let txid = authorized.submit(now, |transaction| {
                let mut child = Process::new(&transport)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()?;
                child
                    .stdin
                    .take()
                    .ok_or(zutti_wallet::Error::Broadcast)?
                    .write_all(alloy::hex::encode(transaction).as_bytes())?;
                let output = child.wait_with_output()?;
                if !output.status.success() {
                    return Err(zutti_wallet::Error::Broadcast);
                }
                let text = std::str::from_utf8(&output.stdout)
                    .map_err(|_| zutti_wallet::Error::Broadcast)?
                    .trim();
                let mut bytes: [u8; 32] = alloy::hex::decode(text)
                    .map_err(|_| zutti_wallet::Error::Broadcast)?
                    .try_into()
                    .map_err(|_| zutti_wallet::Error::Broadcast)?;
                bytes.reverse();
                Ok(Digest(bytes))
            })?;
            println!(
                "{}",
                serde_json::json!({"broadcast":true,"ethereum_reservation_finality_verified":true,"txid":alloy::hex::encode(txid.0.into_iter().rev().collect::<Vec<_>>()),"state":format!("{:?}",journal.state()?)})
            );
        }
        Command::Withdraw {
            inventory,
            position,
            amount,
            owner,
            evidence,
            out,
        } => {
            let inventory = context.inventory(&inventory)?;
            zutti_ethereum::notes::verify_owned(
                &context.profile.ethereum,
                zutti_ethereum::notes::OwnedNoteRequest {
                    domain: &context.profile.domain,
                    code_hash: context.profile.pool_code_hash,
                    inventory: &inventory,
                    keys: context.account.note_keys(),
                    position,
                },
                &read::<NoteEvidence>(&evidence, 16_777_216)?,
                now,
            )?;
            let coin = inventory
                .notes()
                .iter()
                .find(|coin| coin.path.position == position)
                .ok_or("owned note not found")?;
            let amount = Amount::parse(Asset::Usdc, &amount)?;
            let value = u64::try_from(amount.units())?;
            if value == 0
                || owner.is_zero()
                || owner == Address::from(context.profile.domain.pool())
            {
                return Err("invalid withdrawal".into());
            }
            let remaining = coin.note.amount().checked_sub(amount)?;
            let outputs = if remaining.units() == 0 {
                vec![]
            } else {
                vec![NoteOutput {
                    note: zutti_protocol::private_note::PrivateNote::new(
                        context.profile.domain.id()?,
                        context.account.private_receiver()?,
                        remaining,
                        Secret32::random()?,
                        Secret32::random()?,
                    )?,
                    encryption: NoteEncryption::random()?,
                }]
            };
            let claim = SpvPoolClaim {
                policy: context.profile.policy.id()?,
                binding_program: context.profile.binding_program,
                pool_program: context.profile.pool_program,
                expiry: 0,
                pool: PoolClaim {
                    kind: OperationKind::Withdraw,
                    domain: context.profile.domain.id()?,
                    root: inventory.root(),
                    checkpoint: Digest::default(),
                    order_tag: Digest::default(),
                    order: Digest::default(),
                    reservation: Digest::default(),
                    value,
                    owner: owner.into_array(),
                    nullifiers: nullifier_digest(&[context
                        .account
                        .note_keys()
                        .nullifier(&coin.note)?])?,
                    outputs: output_digest(
                        &outputs
                            .iter()
                            .map(NoteOutput::binding)
                            .collect::<zutti_protocol::Result<Vec<_>>>()?,
                    )?,
                },
            };
            let input = NoteInput::authorize(
                zutti_protocol::private_note::PrivateNote::from_bytes(&coin.note.to_bytes()?)?,
                coin.path.clone(),
                context.account.note_keys(),
                claim.journal_digest()?,
            )?;
            let witness = SpvPoolWitness {
                policy: context.profile.policy,
                claim,
                inputs: vec![input],
                outputs,
                action: SpvAction::Withdraw,
            };
            witness.evaluate()?;
            stage(
                &out,
                &[
                    ("witness.bin", witness.to_bytes()?),
                    ("journal.bin", witness.claim.to_bytes()?),
                ],
            )?;
            println!(
                "{}",
                serde_json::json!({"withdrawal_authorized":true,"amount_units":value,"owner":owner,"withdrawn":false})
            );
        }
    }
    Ok(())
}
