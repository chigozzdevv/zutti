use super::{BrowserWallet, Result, clock, decode, profile};
use bincode::Options;
use orchard::keys::{FullViewingKey, Scope};
use risc0_zkvm::{Receipt, VerifierContext, sha::Digestible};
use serde::Deserialize;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;
use zutti_ethereum::{
    notes::{NoteEvidence, NoteRequest, OwnedNoteRequest},
    spv_funding::{NoteFundingEvidence, SpvFundingProfile},
};
use zutti_protocol::{
    Digest, Secret32,
    amount::{Amount, Asset},
    encoding::Canonical,
    pool::{OperationKind, PoolClaim, PoolTree, nullifier_digest, output_digest},
    private_note::PrivateNote,
};
use zutti_wallet::{
    funding::{FundingConsent, SpvFundingPlan},
    inventory::{PoolInventory, PoolOutput},
    pool::{NoteInput, NoteOutput},
    private_note::NoteEncryption,
    spv_pool::{SpvAction, SpvPoolClaim, SpvPoolWitness, SpvReservation, recovery::SpvSwapRecord},
};

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Scan {
        events: Events,
    },
    VerifyNote {
        inventory: String,
        position: u32,
        evidence: Box<NoteEvidence>,
    },
    PrepareFunding {
        inventory: String,
        reservation: String,
    },
    AuthorizeFunding {
        plan: String,
        binding: String,
        source: String,
        headers: Vec<String>,
        evidence: Box<NoteEvidence>,
    },
    VerifyOperation {
        witness: String,
        receipt: String,
    },
    Withdraw {
        inventory: String,
        position: u32,
        evidence: Box<NoteEvidence>,
        amount: String,
        owner: String,
    },
    Window {
        record: String,
        headers: Vec<String>,
        block: String,
    },
    Settle {
        record: String,
        window: String,
        events: Events,
    },
    ReservationRequest {
        record: String,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Events {
    schema: u8,
    chain_id: u64,
    pool: String,
    domain: String,
    root: String,
    next_leaf: u32,
    execution_hash: String,
    execution_height: u64,
    outputs: Vec<Output>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Output {
    position: u32,
    commitment: String,
    ciphertext: String,
}

fn digest(text: &str) -> Result<Digest> {
    Digest::from_hex(text.strip_prefix("0x").unwrap_or(text))
        .map_err(|_| "invalid pool digest".into())
}
fn receipt(text: &str) -> Result<Receipt> {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(super::MAX_RECEIPT as u64)
        .reject_trailing_bytes()
        .deserialize(&decode(text, super::MAX_RECEIPT)?)
        .map_err(|_| "invalid cryptographic receipt".into())
}
fn headers(texts: Vec<String>) -> Result<Vec<Vec<u8>>> {
    if texts.len() > 512 {
        return Err("too many current headers".into());
    }
    texts
        .into_iter()
        .map(|text| {
            let bytes = decode(&text, 1487)?;
            if bytes.len() != 1487 {
                return Err("invalid header length".into());
            }
            Ok(bytes)
        })
        .collect()
}
impl Events {
    fn tree(&self) -> Result<PoolTree> {
        let profile = profile::profile()?;
        let domain = profile.domain.id().map_err(|_| "pool domain")?;
        if self.schema != 1
            || self.chain_id != 11155111
            || digest(&self.domain)? != domain
            || decode(self.pool.strip_prefix("0x").ok_or("pool address")?, 20)?
                != profile.domain.pool()
            || self.outputs.len() > 65_536
            || self.outputs.len() != self.next_leaf as usize
            || decode(
                self.execution_hash
                    .strip_prefix("0x")
                    .ok_or("execution hash")?,
                32,
            )?
            .len()
                != 32
            || self.execution_height == 0
        {
            return Err("pool events deployment mismatch".into());
        }
        let mut tree = PoolTree::new(domain).map_err(|_| "pool tree")?;
        let mut seen = std::collections::BTreeSet::new();
        for (index, output) in self.outputs.iter().enumerate() {
            let commitment = digest(&output.commitment)?;
            if output.position as usize != index
                || !seen.insert(commitment)
                || tree.append(commitment).map_err(|_| "pool insertion")? != output.position
            {
                return Err("incomplete or duplicate pool events".into());
            }
        }
        if tree.root() != digest(&self.root)? {
            return Err("pool events root mismatch".into());
        }
        Ok(tree)
    }
}

#[wasm_bindgen]
impl BrowserWallet {
    pub fn pool(&self, request: &str, now: f64) -> Result<String> {
        if request.len() > super::MAX_REQUEST {
            return Err("pool request exceeds its bound".into());
        }
        let request: Request =
            serde_json::from_str(request).map_err(|_| "invalid pool wallet request")?;
        let now = clock(now)?;
        let profile = profile::profile()?;
        let funding = SpvFundingProfile {
            swap: &profile,
            registry: profile.source,
            registry_code_hash: profile.source_code_hash,
        };
        let result = match request {
            Request::ReservationRequest { record } => {
                let record = SpvSwapRecord::from_bytes(&decode(&record, 262_144)?)
                    .map_err(|_| "swap record")?;
                if record.reservation().payout.note.recipient()
                    != self.notes.recipient().map_err(|_| "payout owner")?
                {
                    return Err("reservation pays another wallet".into());
                }
                serde_json::to_value(
                    profile
                        .reservation_request(&record)
                        .map_err(|_| "reservation profile")?,
                )
                .map_err(|_| "reservation request")?
            }
            Request::Scan { events } => {
                let tree = events.tree()?;
                let outputs = events
                    .outputs
                    .iter()
                    .map(|item| {
                        let output = PoolOutput::from_bytes(&decode(
                            item.ciphertext.strip_prefix("0x").ok_or("ciphertext")?,
                            4096,
                        )?)
                        .map_err(|_| "invalid encrypted output")?;
                        if output.commitment() != digest(&item.commitment)? {
                            return Err("ciphertext commitment mismatch".into());
                        }
                        Ok((item.position, output))
                    })
                    .collect::<Result<Vec<_>>>()?;
                let inventory = PoolInventory::scan_outputs(
                    profile.domain.id().map_err(|_| "domain")?,
                    &self.notes,
                    &outputs,
                    tree.root(),
                    events.next_leaf,
                )
                .map_err(|_| "private inventory scan failed")?;
                let notes = inventory.notes().iter().map(|coin| {
                    let request = NoteRequest { chain_id: 11155111, pool: profile.domain.pool().into(), pool_code_hash: profile.pool_code_hash,
                        root: inventory.root().0.into(), nullifier: self.notes.nullifier(&coin.note).map_err(|_| "note nullifier")?.0.into() };
                    Ok(json!({ "position": coin.path.position, "amount": coin.note.amount().to_string(), "request": request }))
                }).collect::<Result<Vec<_>>>()?;
                json!({ "inventory": self.seal_state(b"pool-inventory", &inventory.to_bytes().map_err(|_| "inventory encoding")?)?,
                    "notes": notes, "unspent_verified": false })
            }
            Request::VerifyNote {
                inventory,
                position,
                evidence,
            } => {
                let inventory = self.inventory(&inventory)?;
                let verified = zutti_ethereum::notes::verify_owned(
                    &profile.ethereum,
                    OwnedNoteRequest {
                        domain: &profile.domain,
                        code_hash: profile.pool_code_hash,
                        inventory: &inventory,
                        keys: &self.notes,
                        position,
                    },
                    &evidence,
                    now,
                )
                .map_err(|_| "owned note finality verification failed")?;
                json!({ "unspent_verified": true, "note": verified })
            }
            Request::PrepareFunding {
                inventory,
                reservation,
            } => {
                let reservation = SpvReservation::from_bytes(&decode(&reservation, 262_144)?)
                    .map_err(|_| "reservation encoding")?;
                self.maker_recipient(&reservation)?;
                let plan = SpvFundingPlan::from_inventory(
                    profile.policy,
                    profile.binding_program,
                    profile.pool_program,
                    &self.inventory(&inventory)?,
                    &self.notes,
                    Box::new(reservation),
                )
                .map_err(|_| "maker funding plan failed")?;
                let request = funding
                    .request(&plan, &self.notes)
                    .map_err(|_| "funding request")?;
                if request.nullifiers.len() != 1 {
                    return Err("single-note funding required".into());
                }
                let note = NoteRequest {
                    chain_id: request.chain_id,
                    pool: request.pool,
                    pool_code_hash: request.pool_code_hash,
                    root: request.root,
                    nullifier: request.nullifiers[0],
                };
                json!({ "plan": self.seal_state(b"funding-plan", &plan.to_bytes().map_err(|_| "plan encoding")?)?,
                    "request": note, "authorized": false })
            }
            Request::AuthorizeFunding {
                plan,
                binding,
                source,
                headers: current,
                evidence,
            } => {
                let plan = SpvFundingPlan::from_bytes(&self.open_state(b"funding-plan", &plan)?)
                    .map_err(|_| "funding plan")?;
                self.maker_recipient(plan.reservation())?;
                let terms = plan.reservation().terms.clone();
                let consent = FundingConsent {
                    terms: &terms,
                    refund: self.notes.recipient().map_err(|_| "maker recipient")?,
                    policy: profile.policy,
                    binding_program: profile.binding_program,
                    pool_program: profile.pool_program,
                };
                let binding = receipt(&binding)?;
                let source = receipt(&source)?;
                let current = headers(current)?;
                let witness = funding
                    .authorize_from_note(
                        plan,
                        &consent,
                        &self.notes,
                        NoteFundingEvidence {
                            ethereum: &evidence,
                            binding: &binding,
                            anchor: &source,
                            headers: &current,
                        },
                        now,
                    )
                    .map_err(
                        |_| "funding proof, note finality or current source verification failed",
                    )?;
                let record =
                    SpvSwapRecord::from_reservation(&witness).map_err(|_| "funding record")?;
                json!({ "witness": hex::encode(witness.to_bytes().map_err(|_| "funding witness")?),
                    "record": hex::encode(record.to_bytes().map_err(|_| "funding record")?),
                    "swap_id": record.id().map_err(|_| "swap identity")?.to_string(),
                    "review": format!("Reserve {} USDC to receive {} ZEC. Public Testnet proving exposes this note's opening and nullifier secret; signing and viewing keys remain in Flask.", terms.usdc(), terms.zec()) })
            }
            Request::VerifyOperation {
                witness,
                receipt: proof,
            } => self.pool_call(&witness, &proof)?,
            Request::Withdraw {
                inventory,
                position,
                evidence,
                amount,
                owner,
            } => {
                let inventory = self.inventory(&inventory)?;
                zutti_ethereum::notes::verify_owned(
                    &profile.ethereum,
                    OwnedNoteRequest {
                        domain: &profile.domain,
                        code_hash: profile.pool_code_hash,
                        inventory: &inventory,
                        keys: &self.notes,
                        position,
                    },
                    &evidence,
                    now,
                )
                .map_err(|_| "withdrawal note finality verification failed")?;
                let coin = inventory
                    .notes()
                    .iter()
                    .find(|coin| coin.path.position == position)
                    .ok_or("owned note not found")?;
                let amount =
                    Amount::parse(Asset::Usdc, &amount).map_err(|_| "withdrawal amount")?;
                let owner: [u8; 20] =
                    decode(owner.strip_prefix("0x").ok_or("withdrawal address")?, 20)?
                        .try_into()
                        .map_err(|_| "withdrawal address")?;
                if amount.units() == 0 || owner == [0; 20] || owner == profile.domain.pool() {
                    return Err("invalid withdrawal".into());
                }
                let remaining = coin
                    .note
                    .amount()
                    .checked_sub(amount)
                    .map_err(|_| "insufficient private balance")?;
                let outputs = if remaining.units() == 0 {
                    vec![]
                } else {
                    vec![NoteOutput {
                        note: PrivateNote::new(
                            profile.domain.id().map_err(|_| "domain")?,
                            self.notes.recipient().map_err(|_| "recipient")?,
                            remaining,
                            Secret32::random().map_err(|_| "note randomness")?,
                            Secret32::random().map_err(|_| "note randomness")?,
                        )
                        .map_err(|_| "change note")?,
                        encryption: NoteEncryption::random()
                            .map_err(|_| "encryption randomness")?,
                    }]
                };
                let claim = SpvPoolClaim {
                    policy: profile.policy.id().map_err(|_| "policy")?,
                    binding_program: profile.binding_program,
                    pool_program: profile.pool_program,
                    expiry: 0,
                    pool: PoolClaim {
                        kind: OperationKind::Withdraw,
                        domain: profile.domain.id().map_err(|_| "domain")?,
                        root: inventory.root(),
                        checkpoint: Digest::default(),
                        order_tag: Digest::default(),
                        order: Digest::default(),
                        reservation: Digest::default(),
                        value: u64::try_from(amount.units()).map_err(|_| "withdrawal amount")?,
                        owner,
                        nullifiers: nullifier_digest(&[self
                            .notes
                            .nullifier(&coin.note)
                            .map_err(|_| "nullifier")?])
                        .map_err(|_| "nullifiers")?,
                        outputs: output_digest(
                            &outputs
                                .iter()
                                .map(NoteOutput::binding)
                                .collect::<zutti_protocol::Result<Vec<_>>>()
                                .map_err(|_| "output binding")?,
                        )
                        .map_err(|_| "output digest")?,
                    },
                };
                let input = NoteInput::authorize(
                    PrivateNote::from_bytes(&coin.note.to_bytes().map_err(|_| "note encoding")?)
                        .map_err(|_| "note encoding")?,
                    coin.path.clone(),
                    &self.notes,
                    claim.journal_digest().map_err(|_| "operation digest")?,
                )
                .map_err(|_| "note authorization")?;
                let witness = SpvPoolWitness {
                    policy: profile.policy,
                    claim,
                    inputs: vec![input],
                    outputs,
                    action: SpvAction::Withdraw,
                };
                witness.evaluate().map_err(|_| "withdrawal conservation")?;
                json!({ "witness": hex::encode(witness.to_bytes().map_err(|_| "withdrawal witness")?),
                    "review": format!("Withdraw {amount} USDC publicly to 0x{}. Public Testnet proving exposes note openings and the nullifier secret.", hex::encode(owner)) })
            }
            Request::Window {
                record,
                headers: current,
                block,
            } => {
                use zutti_source::spv::{Branch, WindowWitness, Witness};
                let record = SpvSwapRecord::from_bytes(&decode(&record, 262_144)?)
                    .map_err(|_| "swap record")?;
                profile
                    .reservation_request(&record)
                    .map_err(|_| "swap profile")?;
                let reservation = record.reservation();
                let current = headers(current)?;
                let end = reservation
                    .anchor
                    .advance(&current)
                    .map_err(|_| "payment headers")?;
                let window = WindowWitness::new(
                    reservation.window().map_err(|_| "payment window")?,
                    reservation.txid,
                    Secret32::from_bytes(*reservation.reservation_randomness.expose())
                        .map_err(|_| "reservation randomness")?,
                    Branch::Included,
                    current,
                    vec![decode(&block, 2_000_000)?],
                )
                .map_err(|_| "payment window witness")?;
                let witness = Witness::Window {
                    previous: reservation.anchor.clone(),
                    window: Box::new(window),
                };
                json!({ "witness": hex::encode(witness.to_bytes().map_err(|_| "window encoding")?),
                    "journal": hex::encode(witness.evaluate().map_err(|_| "payment inclusion")?.journal().map_err(|_| "window journal")?),
                    "end_header": hex::encode(end.to_bytes().map_err(|_| "end header")?) })
            }
            Request::Settle {
                record,
                window,
                events,
            } => {
                let record = SpvSwapRecord::from_bytes(&decode(&record, 262_144)?)
                    .map_err(|_| "swap record")?;
                profile
                    .reservation_request(&record)
                    .map_err(|_| "swap profile")?;
                let window = zutti_source::spv::WindowClaim::from_bytes(&decode(&window, 4096)?)
                    .map_err(|_| "window claim")?;
                if window.output != record.reservation().terms.payout().commitment() {
                    return Err("only paid settlement is enabled".into());
                }
                let tree = events.tree()?;
                let escrow = record
                    .reservation()
                    .escrow
                    .commitment()
                    .map_err(|_| "escrow commitment")?;
                let position = events
                    .outputs
                    .iter()
                    .find(|out| digest(&out.commitment).ok() == Some(escrow))
                    .ok_or("reserved escrow not found")?
                    .position;
                let witness = record
                    .settlement(
                        tree.root(),
                        tree.prove(position).map_err(|_| "escrow path")?,
                        window,
                    )
                    .map_err(|_| "payout settlement")?;
                json!({ "witness": hex::encode(witness.to_bytes().map_err(|_| "settlement encoding")?) })
            }
        };
        Ok(result.to_string())
    }
}

impl BrowserWallet {
    fn inventory(&self, state: &str) -> Result<PoolInventory> {
        PoolInventory::from_bytes(&self.open_state(b"pool-inventory", state)?)
            .map_err(|_| "invalid saved inventory".into())
    }
    fn maker_recipient(&self, reservation: &SpvReservation) -> Result<()> {
        if reservation.terms.receiver()
            != &FullViewingKey::from(&self.source)
                .address_at(0_u32, Scope::External)
                .to_raw_address_bytes()
            || reservation.refund.note.recipient()
                != self.notes.recipient().map_err(|_| "maker recipient")?
        {
            return Err("maker receiving address mismatch".into());
        }
        Ok(())
    }
    fn pool_call(&self, witness: &str, proof: &str) -> Result<Value> {
        let witness =
            SpvPoolWitness::from_bytes(&decode(witness, 262_144)?).map_err(|_| "pool witness")?;
        let profile = profile::profile()?;
        let evaluated = witness.evaluate().map_err(|_| "pool operation")?;
        if evaluated.policy != profile.policy.id().map_err(|_| "policy")?
            || evaluated.binding_program != profile.binding_program
            || evaluated.pool_program != profile.pool_program
            || evaluated.pool.domain != profile.domain.id().map_err(|_| "domain")?
        {
            return Err("pool operation deployment mismatch".into());
        }
        let owner = self.notes.recipient().map_err(|_| "note owner")?;
        match &witness.action {
            SpvAction::Reserve(reservation) => {
                self.maker_recipient(reservation)?;
                if witness.inputs.is_empty()
                    || witness
                        .inputs
                        .iter()
                        .any(|input| input.note.recipient() != owner)
                {
                    return Err("foreign reservation input".into());
                }
            }
            SpvAction::Settle { .. } => {
                if witness.outputs.len() != 1 || witness.outputs[0].note.recipient() != owner {
                    return Err("foreign private payout".into());
                }
            }
            SpvAction::Withdraw => {
                if witness.inputs.is_empty()
                    || witness
                        .inputs
                        .iter()
                        .any(|input| input.note.recipient() != owner)
                {
                    return Err("foreign withdrawal input".into());
                }
            }
            _ => return Err("unsupported browser operation".into()),
        }
        let proof = receipt(proof)?;
        proof
            .verify_with_context(
                &VerifierContext::default().with_dev_mode(false),
                profile.pool_program.0,
            )
            .map_err(|_| "pool proof verification failed")?;
        if proof.journal.bytes != evaluated.to_bytes().map_err(|_| "pool journal")? {
            return Err("pool proof journal mismatch".into());
        }
        let risc0_zkvm::InnerReceipt::Groth16(inner) = &proof.inner else {
            return Err("Ethereum-compatible pool proof required".into());
        };
        let parameters = risc0_zkvm::Groth16ReceiptVerifierParameters::default().digest();
        if inner.verifier_parameters != parameters || inner.seal.len() != 256 {
            return Err("unsupported Ethereum verifier".into());
        }
        let mut seal = parameters.as_bytes()[..4].to_vec();
        seal.extend_from_slice(&inner.seal);
        let mut nullifiers = witness
            .inputs
            .iter()
            .map(|input| input.note.nullifier(&input.nullifier_key))
            .collect::<zutti_protocol::Result<Vec<_>>>()
            .map_err(|_| "nullifiers")?;
        let mut outputs = Vec::new();
        let mut record = None;
        match &witness.action {
            SpvAction::Reserve(reservation) => {
                outputs.push((
                    reservation.escrow.commitment().map_err(|_| "escrow")?,
                    reservation
                        .escrow
                        .encrypt(reservation.nonce)
                        .map_err(|_| "escrow encryption")?,
                ));
                let saved = SpvSwapRecord::from_reservation(&witness)
                    .map_err(|_| "swap record")?
                    .with_funding_seal(seal.clone())
                    .map_err(|_| "funding seal")?;
                record = Some(hex::encode(saved.to_bytes().map_err(|_| "swap record")?));
            }
            SpvAction::Settle { reservation, .. } => nullifiers.push(
                reservation
                    .escrow
                    .nullifier()
                    .map_err(|_| "escrow nullifier")?,
            ),
            _ => {}
        }
        for output in &witness.outputs {
            outputs.push((
                output.note.commitment().map_err(|_| "output commitment")?,
                output
                    .encrypted()
                    .map_err(|_| "output encryption")?
                    .to_bytes()
                    .map_err(|_| "output encoding")?,
            ));
        }
        let data = encode_call(&seal, &evaluated, &nullifiers, &outputs);
        Ok(
            json!({ "chain_id": 11155111, "to": format!("0x{}", hex::encode(profile.domain.pool())), "value": "0x0",
            "data": format!("0x{}", hex::encode(data)), "kind": evaluated.pool.kind as u8, "record": record,
            "review": if evaluated.pool.kind == OperationKind::Withdraw { format!("Submit the verified withdrawal of {} USDC to 0x{}.", Amount::from_units(Asset::Usdc, u128::from(evaluated.pool.value)).map_err(|_| "withdrawal amount")?, hex::encode(evaluated.pool.owner)) }
                else { "Submit this verified Zutti pool operation on Sepolia.".to_string() } }),
        )
    }
}

fn word(value: u64) -> [u8; 32] {
    let mut bytes = [0; 32];
    bytes[24..].copy_from_slice(&value.to_be_bytes());
    bytes
}
fn dynamic(bytes: &[u8]) -> Vec<u8> {
    let mut out = word(bytes.len() as u64).to_vec();
    out.extend_from_slice(bytes);
    out.resize(32 + bytes.len().div_ceil(32) * 32, 0);
    out
}
fn encode_call(
    seal: &[u8],
    claim: &SpvPoolClaim,
    nullifiers: &[Digest],
    outputs: &[(Digest, Vec<u8>)],
) -> Vec<u8> {
    let proof = dynamic(seal);
    let pool = &claim.pool;
    let mut out = vec![0x1d, 0x0a, 0x9b, 0x2e];
    out.extend_from_slice(&word(64));
    out.extend_from_slice(&word(64 + proof.len() as u64));
    out.extend_from_slice(&proof);
    out.extend_from_slice(&word(pool.kind as u64));
    for value in [pool.root, pool.checkpoint] {
        out.extend_from_slice(&value.0);
    }
    out.extend_from_slice(&word(u64::from(claim.expiry)));
    for value in [pool.order_tag, pool.order, pool.reservation] {
        out.extend_from_slice(&value.0);
    }
    out.extend_from_slice(&word(pool.value));
    out.extend_from_slice(&[0; 12]);
    out.extend_from_slice(&pool.owner);
    out.extend_from_slice(&word(352));
    out.extend_from_slice(&word(384 + 32 * nullifiers.len() as u64));
    out.extend_from_slice(&word(nullifiers.len() as u64));
    for value in nullifiers {
        out.extend_from_slice(&value.0);
    }
    out.extend_from_slice(&word(outputs.len() as u64));
    let mut offset = outputs.len() as u64 * 32;
    for (_, ciphertext) in outputs {
        out.extend_from_slice(&word(offset));
        offset += 64 + dynamic(ciphertext).len() as u64;
    }
    for (commitment, ciphertext) in outputs {
        out.extend_from_slice(&commitment.0);
        out.extend_from_slice(&word(64));
        out.extend_from_slice(&dynamic(ciphertext));
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use sha2::{Digest as _, Sha256};
    #[test]
    fn pool_calldata_matches_independent_abi_encoding() {
        let claim = SpvPoolClaim {
            policy: Digest([1; 32]),
            binding_program: Digest([1; 32]),
            pool_program: Digest([1; 32]),
            expiry: 0,
            pool: PoolClaim {
                kind: OperationKind::Settle,
                domain: Digest([1; 32]),
                root: Digest([2; 32]),
                checkpoint: Digest([3; 32]),
                order_tag: Digest::default(),
                order: Digest::default(),
                reservation: Digest::default(),
                value: 0,
                owner: [0; 20],
                nullifiers: Digest::default(),
                outputs: Digest::default(),
            },
        };
        let call = encode_call(
            &[1; 260],
            &claim,
            &[Digest([4; 32]), Digest([5; 32])],
            &[
                (Digest([6; 32]), vec![7; 39]),
                (Digest([8; 32]), vec![9; 65]),
            ],
        );
        assert_eq!(call.len(), 1284);
        assert_eq!(
            hex::encode(Sha256::digest(call)),
            "f36f62a1ff166546807378301db6e21481bd262214253f82c8458f6cbbe3869d"
        );
    }
    #[test]
    fn encrypted_pool_inventory_is_private_and_cannot_be_faked_as_finality() {
        let wallet = BrowserWallet::new(vec![42; 32]).unwrap();
        let deposit: Value = serde_json::from_str(
            &wallet
                .prepare_deposit("15", &format!("0x{}", "22".repeat(20)))
                .unwrap(),
        )
        .unwrap();
        let witness =
            SpvPoolWitness::from_bytes(&hex::decode(deposit["witness"].as_str().unwrap()).unwrap())
                .unwrap();
        let output = &witness.outputs[0];
        let domain = profile::profile().unwrap().domain.id().unwrap();
        let mut tree = PoolTree::new(domain).unwrap();
        tree.append(output.note.commitment().unwrap()).unwrap();
        let events = json!({ "schema":1,"chain_id":11155111,"pool":deposit["pool"],"domain":domain.to_string(),"root":tree.root().to_string(),
            "next_leaf":1,"execution_hash":format!("0x{}", "11".repeat(32)),"execution_height":1,
            "outputs":[{"position":0,"commitment":output.note.commitment().unwrap().to_string(),"ciphertext":format!("0x{}",hex::encode(output.encrypted().unwrap().to_bytes().unwrap()))}] });
        let result: Value = serde_json::from_str(
            &wallet
                .pool(
                    &json!({"action":"scan", "events": events}).to_string(),
                    1793200000.0,
                )
                .unwrap(),
        )
        .unwrap();
        assert_eq!(result["notes"][0]["amount"], "15");
        assert_eq!(result["unspent_verified"], false);
        assert!(
            BrowserWallet::new(vec![43; 32])
                .unwrap()
                .inventory(result["inventory"].as_str().unwrap())
                .is_err()
        );
        assert!(wallet.pool(&json!({"action":"verify_note", "inventory":result["inventory"],"position":0,"evidence":{"verified":true}}).to_string(),1793200000.0).is_err());
        let mut altered = events.clone();
        altered["outputs"][0]["position"] = json!(1);
        assert!(
            wallet
                .pool(
                    &json!({"action":"scan", "events": altered}).to_string(),
                    1793200000.0
                )
                .is_err()
        );
    }
}
