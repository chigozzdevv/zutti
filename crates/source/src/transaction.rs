use crate::authenticated::{AuthenticatedMap, StateError, Transcript};
use crate::block::ParsedBlock;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use transparent::{
    address::Script,
    bundle::{self, OutPoint, TxOut},
    sighash::{SighashType, SignableInput as TransparentInput, TransparentAuthorizingContext},
};
use zcash_encoding::CompactSize;
use zcash_primitives::transaction::{
    Authorization, Transaction, TransactionData, TxVersion,
    sighash::{SignableInput, signature_hash},
    txid::TxIdDigester,
};
use zcash_protocol::{consensus::BranchId, value::Zatoshis};
use zcash_script::{
    interpreter::{CallbackTransactionSignatureChecker, Flags},
    script::{self, Evaluable as _},
};
use zutti_protocol::{
    Digest,
    commitment::hash,
    encoding::{Canonical, Reader, Writer},
};

pub const MAX_MONEY: u64 = 21_000_000 * 100_000_000;
pub const COINBASE_MATURITY: u32 = 100;
pub const MAX_BLOCK_SIGOPS: u32 = 20_000;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error(transparent)]
    State(#[from] StateError),
    #[error(transparent)]
    Authorization(#[from] crate::authorization::Error),
    #[error("transaction version is not permitted in this consensus branch")]
    Version,
    #[error("transaction has no effective inputs or outputs")]
    Empty,
    #[error("transaction is expired or has an invalid expiry height")]
    Expiry,
    #[error("transaction lock time has not passed")]
    LockTime,
    #[error("invalid coinbase transaction")]
    Coinbase,
    #[error("coinbase shielded output cannot be recovered with the consensus outgoing viewing key")]
    CoinbaseCiphertext,
    #[error("transparent input is missing, already spent or duplicated")]
    Input,
    #[error("coinbase input is immature or has transparent outputs")]
    CoinbaseSpend,
    #[error("transparent input script or signature is invalid")]
    Script,
    #[error("transaction amounts are out of range or do not conserve value")]
    Value,
    #[error("transaction would overwrite an unspent output")]
    Overwrite,
    #[error("block exceeds the consensus signature operation limit")]
    Sigops,
    #[error("transaction serialization failed")]
    Encoding,
}
pub type Result<T> = core::result::Result<T, Error>;

#[derive(Debug, Clone)]
pub struct UnspentOutput {
    output: TxOut,
    height: u32,
    coinbase: bool,
}
impl UnspentOutput {
    pub fn output(&self) -> &TxOut {
        &self.output
    }
    pub const fn height(&self) -> u32 {
        self.height
    }
    pub const fn coinbase(&self) -> bool {
        self.coinbase
    }
}

type OutputKey = ([u8; 32], u32);

#[derive(Debug, Clone)]
pub struct CandidateLedger {
    pub(crate) outputs: AuthenticatedMap,
    pub(crate) count: u64,
}
impl Default for CandidateLedger {
    fn default() -> Self {
        Self {
            outputs: AuthenticatedMap::new(ledger_context()),
            count: 0,
        }
    }
}
pub(crate) fn ledger_context() -> Digest {
    hash(b"source-utxos", &[])
}
fn output_key(key: OutputKey) -> Digest {
    hash(b"source-outpoint", &[&key.0, &key.1.to_le_bytes()])
}
impl Canonical for UnspentOutput {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        writer.u32(self.height);
        writer.u8(u8::from(self.coinbase));
        writer.u64(self.output.value().into_u64());
        writer.bytes(&self.output.script_pubkey().0.0)
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        let height = reader.u32()?;
        let coinbase = match reader.u8()? {
            0 => false,
            1 => true,
            _ => return Err(zutti_protocol::Error::Invalid("coinbase flag")),
        };
        let value = Zatoshis::from_u64(reader.u64()?)
            .map_err(|_| zutti_protocol::Error::Invalid("utxo value"))?;
        let script = Script(script::Code(reader.bytes(10_000)?.to_vec()));
        Ok(Self {
            output: TxOut::new(value, script),
            height,
            coinbase,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockValue {
    pub fees: u64,
    pub coinbase_value: u64,
    pub sigops: u32,
}

impl CandidateLedger {
    pub fn output(&self, outpoint: &OutPoint) -> Result<Option<UnspentOutput>> {
        self.outputs
            .get(output_key(key(outpoint)))?
            .map(|bytes| UnspentOutput::from_bytes(&bytes).map_err(|_| Error::Encoding))
            .transpose()
    }
    pub fn len(&self) -> usize {
        self.count as usize
    }
    pub const fn count(&self) -> u64 {
        self.count
    }
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }
    pub fn root(&self) -> Digest {
        self.outputs.root()
    }
    pub fn start_recording(&mut self) -> Result<()> {
        Ok(self.outputs.start_recording()?)
    }
    pub fn take_transcript(&mut self) -> Result<Transcript> {
        Ok(self.outputs.take_transcript()?)
    }
    pub fn from_witness(root: Digest, count: u64, transcript: Transcript) -> Self {
        Self {
            outputs: AuthenticatedMap::from_witness(ledger_context(), root, transcript),
            count,
        }
    }
    pub fn finish_witness(&self) -> Result<()> {
        Ok(self.outputs.finish()?)
    }
    fn insert_output(&mut self, key: OutputKey, output: UnspentOutput) -> Result<()> {
        let bytes = output.to_bytes().map_err(|_| Error::Encoding)?;
        if self.outputs.set(output_key(key), Some(bytes))?.is_some() {
            return Err(Error::Overwrite);
        }
        self.count = self.count.checked_add(1).ok_or(Error::Value)?;
        Ok(())
    }
    pub fn append(&mut self, block: &ParsedBlock) -> Result<BlockValue> {
        let mut next = self.clone();
        let mut value = BlockValue {
            fees: 0,
            coinbase_value: 0,
            sigops: 0,
        };
        for (position, transaction) in block.transactions().iter().enumerate() {
            let coinbase = position == 0;
            check_context(
                transaction,
                block.branch(),
                block.claimed_height(),
                block.header().time,
                coinbase,
            )?;
            let result = next.apply(transaction, block.claimed_height(), coinbase)?;
            if coinbase {
                value.coinbase_value = result.0;
            } else {
                value.fees = money_add(value.fees, result.0)?;
            }
            value.sigops = value.sigops.checked_add(result.1).ok_or(Error::Sigops)?;
            if value.sigops > MAX_BLOCK_SIGOPS {
                return Err(Error::Sigops);
            }
        }
        *self = next;
        Ok(value)
    }
    fn apply(&mut self, tx: &Transaction, height: u32, coinbase: bool) -> Result<(u64, u32)> {
        let mut prevouts = Vec::new();
        let mut spent = BTreeSet::new();
        let mut sigops = 0_u32;
        let mut input_value = 0_u64;
        let mut output_value = 0_u64;
        if let Some(bundle) = tx.transparent_bundle() {
            for input in &bundle.vin {
                sigops = sigops
                    .checked_add(input.script_sig().0.sig_op_count(false))
                    .ok_or(Error::Sigops)?;
                if !coinbase {
                    if !spent.insert(key(input.prevout())) {
                        return Err(Error::Input);
                    }
                    let previous = self.output(input.prevout())?.ok_or(Error::Input)?;
                    if previous.coinbase
                        && (height
                            .checked_sub(previous.height)
                            .is_none_or(|depth| depth < COINBASE_MATURITY)
                            || !bundle.vout.is_empty())
                    {
                        return Err(Error::CoinbaseSpend);
                    }
                    input_value = money_add(input_value, previous.output.value().into_u64())?;
                    prevouts.push(previous.output.clone());
                }
            }
            for output in &bundle.vout {
                output_value = money_add(output_value, output.value().into_u64())?;
                sigops = sigops
                    .checked_add(output.script_pubkey().0.sig_op_count(false))
                    .ok_or(Error::Sigops)?;
            }
            if !coinbase {
                for (index, (input, previous)) in bundle.vin.iter().zip(&prevouts).enumerate() {
                    let scripts = script::Raw {
                        sig: input.script_sig().0.clone(),
                        pub_key: previous.script_pubkey().0.clone(),
                    };
                    let hash =
                        |code: &script::Code, hash_type: &zcash_script::signature::HashType| {
                            input_signature_hash(
                                tx,
                                &prevouts,
                                Some(index),
                                code,
                                hash_type.raw_bits() as u8,
                            )
                            .ok()
                        };
                    let checker = CallbackTransactionSignatureChecker {
                        sighash: &hash,
                        lock_time: i64::from(tx.lock_time()),
                        is_final: input.sequence() == u32::MAX,
                    };
                    if !scripts
                        .eval(Flags::P2SH | Flags::CHECKLOCKTIMEVERIFY, &checker)
                        .map_err(|_| Error::Script)?
                    {
                        return Err(Error::Script);
                    }
                    if scripts.pub_key.is_pay_to_script_hash() {
                        let redeem = scripts
                            .sig
                            .parse()
                            .last()
                            .and_then(|op| op.ok())
                            .and_then(|op| match op {
                                zcash_script::opcode::PossiblyBad::Good(
                                    zcash_script::Opcode::PushValue(value),
                                ) => Some(value.value()),
                                _ => None,
                            })
                            .ok_or(Error::Script)?;
                        sigops = sigops
                            .checked_add(script::Code(redeem).sig_op_count(true))
                            .ok_or(Error::Sigops)?;
                    }
                }
            }
        }
        if let Some(bundle) = tx.sprout_bundle() {
            for js in &bundle.joinsplits {
                let old = u64::try_from(i64::from(js.vpub_old())).map_err(|_| Error::Value)?;
                let new = u64::try_from(i64::from(js.vpub_new())).map_err(|_| Error::Value)?;
                if old != 0 && new != 0 {
                    return Err(Error::Value);
                }
                output_value = money_add(output_value, old)?;
                input_value = money_add(input_value, new)?;
            }
        }
        let balances = [
            tx.sapling_bundle()
                .map_or(0, |b| i64::from(*b.value_balance())),
            tx.orchard_bundle()
                .map_or(0, |b| i64::from(*b.value_balance())),
            tx.ironwood_bundle()
                .map_or(0, |b| i64::from(*b.value_balance())),
        ];
        for balance in balances {
            if balance < 0 {
                output_value = money_add(output_value, balance.unsigned_abs())?;
            } else {
                input_value = money_add(input_value, balance as u64)?;
            }
        }
        let value = if coinbase {
            if input_value != 0 {
                return Err(Error::Coinbase);
            }
            output_value
        } else {
            input_value.checked_sub(output_value).ok_or(Error::Value)?
        };
        crate::authorization::verify(tx, &prevouts)?;
        for outpoint in spent {
            if self.outputs.set(output_key(outpoint), None)?.is_none() {
                return Err(Error::Input);
            }
            self.count = self.count.checked_sub(1).ok_or(Error::Input)?;
        }
        if let Some(bundle) = tx.transparent_bundle() {
            for (index, output) in bundle.vout.iter().enumerate() {
                let outpoint = (
                    *tx.txid().as_ref(),
                    u32::try_from(index).map_err(|_| Error::Encoding)?,
                );
                if self.outputs.get(output_key(outpoint))?.is_some() {
                    return Err(Error::Overwrite);
                }
                if !output.script_pubkey().0.is_unspendable() {
                    self.insert_output(
                        outpoint,
                        UnspentOutput {
                            output: output.clone(),
                            height,
                            coinbase,
                        },
                    )?;
                }
            }
        }
        Ok((value, sigops))
    }
}

pub fn check_context(
    tx: &Transaction,
    branch: BranchId,
    height: u32,
    time: u32,
    coinbase: bool,
) -> Result<()> {
    if matches!(branch, BranchId::Sprout | BranchId::Overwinter) {
        let mut serialized = Vec::new();
        tx.write(&mut serialized).map_err(|_| Error::Encoding)?;
        if serialized.len() > 100_000 {
            return Err(Error::Encoding);
        }
    }
    if !tx.version().valid_in_branch(branch)
        || matches!(tx.version(), TxVersion::Sprout(version) if version < 1)
        || (tx.version().has_overwinter() && tx.consensus_branch_id() != branch)
    {
        return Err(Error::Version);
    }
    let transparent = tx.transparent_bundle();
    if transparent.is_some_and(|bundle| bundle.is_coinbase()) != coinbase {
        return Err(Error::Coinbase);
    }
    let expiry = u32::from(tx.expiry_height());
    if tx.version().has_overwinter()
        && (expiry >= 500_000_000 || (!coinbase && expiry != 0 && height > expiry))
    {
        return Err(Error::Expiry);
    }
    if coinbase {
        let script = transparent
            .and_then(|b| b.vin.first())
            .ok_or(Error::Coinbase)?
            .script_sig();
        if !(2..=100).contains(&script.0.0.len())
            || tx.sprout_bundle().is_some()
            || tx
                .sapling_bundle()
                .is_some_and(|b| !b.shielded_spends().is_empty())
            || tx
                .orchard_bundle()
                .is_some_and(|b| b.flags().spends_enabled())
            || tx
                .ironwood_bundle()
                .is_some_and(|b| b.flags().spends_enabled())
        {
            return Err(Error::Coinbase);
        }
        if matches!(
            branch,
            BranchId::Sprout | BranchId::Overwinter | BranchId::Sapling | BranchId::Blossom
        ) && tx.sapling_bundle().is_some()
        {
            return Err(Error::Coinbase);
        }
        if matches!(
            branch,
            BranchId::Nu5 | BranchId::Nu6 | BranchId::Nu6_1 | BranchId::Nu6_2 | BranchId::Nu6_3
        ) && expiry != height
        {
            return Err(Error::Expiry);
        }
        if let Some(bundle) = tx.sapling_bundle() {
            let enforcement = if branch == BranchId::Heartwood {
                sapling::note_encryption::Zip212Enforcement::Off
            } else {
                sapling::note_encryption::Zip212Enforcement::On
            };
            for output in bundle.shielded_outputs() {
                if sapling::note_encryption::try_sapling_output_recovery(
                    &sapling::keys::OutgoingViewingKey([0; 32]),
                    output,
                    enforcement,
                )
                .is_none()
                {
                    return Err(Error::CoinbaseCiphertext);
                }
            }
        }
        for bundle in [tx.orchard_bundle(), tx.ironwood_bundle()]
            .into_iter()
            .flatten()
        {
            let key = orchard::keys::OutgoingViewingKey::from([0; 32]);
            for index in 0..bundle.actions().len() {
                if bundle.recover_output_with_ovk(index, &key).is_none() {
                    return Err(Error::CoinbaseCiphertext);
                }
            }
        }
    }
    let transparent_inputs = transparent.is_some_and(|b| !b.vin.is_empty());
    let transparent_outputs = transparent.is_some_and(|b| !b.vout.is_empty());
    let joinsplits = tx.sprout_bundle().is_some_and(|b| !b.joinsplits.is_empty());
    let orchard_spends = tx
        .orchard_bundle()
        .is_some_and(|b| b.flags().spends_enabled())
        || tx
            .ironwood_bundle()
            .is_some_and(|b| b.flags().spends_enabled());
    let orchard_outputs = tx
        .orchard_bundle()
        .is_some_and(|b| b.flags().outputs_enabled())
        || tx
            .ironwood_bundle()
            .is_some_and(|b| b.flags().outputs_enabled());
    if !(transparent_inputs
        || joinsplits
        || orchard_spends
        || tx
            .sapling_bundle()
            .is_some_and(|b| !b.shielded_spends().is_empty()))
        || !(transparent_outputs
            || joinsplits
            || orchard_outputs
            || tx
                .sapling_bundle()
                .is_some_and(|b| !b.shielded_outputs().is_empty()))
    {
        return Err(Error::Empty);
    }
    if tx.lock_time() != 0
        && tx.lock_time()
            >= if tx.lock_time() < 500_000_000 {
                height
            } else {
                time
            }
        && !transparent.is_none_or(|b| b.vin.iter().all(|input| input.sequence() == u32::MAX))
    {
        return Err(Error::LockTime);
    }
    if matches!(
        branch,
        BranchId::Canopy
            | BranchId::Nu5
            | BranchId::Nu6
            | BranchId::Nu6_1
            | BranchId::Nu6_2
            | BranchId::Nu6_3
    ) && tx
        .sprout_bundle()
        .is_some_and(|b| b.joinsplits.iter().any(|js| i64::from(js.vpub_old()) != 0))
    {
        return Err(Error::Value);
    }
    if branch == BranchId::Nu6_3
        && tx
            .orchard_bundle()
            .is_some_and(|b| b.flags().cross_address_enabled())
    {
        return Err(Error::Version);
    }
    if branch == BranchId::Nu6_3
        && tx
            .orchard_bundle()
            .is_some_and(|b| i64::from(*b.value_balance()) < 0 || coinbase)
    {
        return Err(Error::Value);
    }
    Ok(())
}

#[derive(Debug)]
struct InputContext(Vec<TxOut>);
impl bundle::Authorization for InputContext {
    type ScriptSig = Script;
}
impl TransparentAuthorizingContext for InputContext {
    fn input_amounts(&self) -> Vec<Zatoshis> {
        self.0.iter().map(TxOut::value).collect()
    }
    fn input_scriptpubkeys(&self) -> Vec<Script> {
        self.0.iter().map(|o| o.script_pubkey().clone()).collect()
    }
}
#[derive(Debug)]
struct Contextual;
impl Authorization for Contextual {
    type TransparentAuth = InputContext;
    type SaplingAuth = sapling::bundle::Authorized;
    type OrchardAuth = orchard::bundle::Authorized;
}

pub fn input_signature_hash(
    tx: &Transaction,
    prevouts: &[TxOut],
    input: Option<usize>,
    code: &script::Code,
    hash_type: u8,
) -> Result<[u8; 32]> {
    if let TxVersion::Sprout(version) = tx.version() {
        return legacy_hash(tx, version, input, code, u32::from(hash_type));
    }
    if !tx
        .transparent_bundle()
        .is_none_or(|bundle| bundle.is_coinbase() || bundle.vin.len() == prevouts.len())
    {
        return Err(Error::Input);
    }
    let data: TransactionData<Contextual> = tx.clone().into_data().map_bundles(
        |bundle| {
            bundle.map(|b| bundle::Bundle {
                vin: b
                    .vin
                    .into_iter()
                    .map(|input| {
                        bundle::TxIn::from_parts(
                            input.prevout().clone(),
                            input.script_sig().clone(),
                            input.sequence(),
                        )
                    })
                    .collect(),
                vout: b.vout,
                authorization: InputContext(prevouts.to_vec()),
            })
        },
        |b| b,
        |b| b,
    );
    let script_code = Script(code.clone());
    let hash_type = if matches!(tx.version(), TxVersion::V5 | TxVersion::V6) {
        SighashType::parse(hash_type).ok_or(Error::Script)?
    } else {
        SighashType::from_raw(hash_type)
    };
    let signable = match input {
        Some(index) => {
            let bundle = data.transparent_bundle().ok_or(Error::Input)?;
            let previous = prevouts.get(index).ok_or(Error::Input)?;
            if matches!(tx.version(), TxVersion::V5 | TxVersion::V6)
                && hash_type.encode() & 0x1f == 3
                && index >= bundle.vout.len()
            {
                return Err(Error::Script);
            }
            SignableInput::Transparent(
                TransparentInput::from_parts(
                    bundle,
                    hash_type,
                    index,
                    &script_code,
                    previous.script_pubkey(),
                    previous.value(),
                )
                .map_err(|_| Error::Input)?,
            )
        }
        None => SignableInput::Shielded,
    };
    Ok(*signature_hash(&data, &signable, &data.digest(TxIdDigester)).as_ref())
}

fn legacy_hash(
    tx: &Transaction,
    version: u32,
    input: Option<usize>,
    code: &script::Code,
    hash_type: u32,
) -> Result<[u8; 32]> {
    let inputs = tx
        .transparent_bundle()
        .map_or(&[][..], |b| b.vin.as_slice());
    let outputs = tx
        .transparent_bundle()
        .map_or(&[][..], |b| b.vout.as_slice());
    let single = hash_type & 0x1f == 3;
    let none = hash_type & 0x1f == 2;
    let anyone = hash_type & 0x80 != 0;
    if input.is_some_and(|index| index >= inputs.len() || (single && index >= outputs.len()))
        || (input.is_none() && (single || none || anyone))
    {
        return Err(Error::Script);
    }
    let mut encoded = Vec::new();
    encoded.extend_from_slice(&version.to_le_bytes());
    CompactSize::write(&mut encoded, if anyone { 1 } else { inputs.len() })
        .map_err(|_| Error::Encoding)?;
    for (index, txin) in inputs
        .iter()
        .enumerate()
        .filter(|(i, _)| !anyone || Some(*i) == input)
    {
        txin.prevout()
            .write(&mut encoded)
            .map_err(|_| Error::Encoding)?;
        Script(if Some(index) == input {
            code.clone()
        } else {
            script::Code(vec![])
        })
        .write(&mut encoded)
        .map_err(|_| Error::Encoding)?;
        encoded.extend_from_slice(
            &if Some(index) != input && (single || none) {
                0
            } else {
                txin.sequence()
            }
            .to_le_bytes(),
        );
    }
    let count = if none {
        0
    } else if single {
        input.ok_or(Error::Script)? + 1
    } else {
        outputs.len()
    };
    CompactSize::write(&mut encoded, count).map_err(|_| Error::Encoding)?;
    for (index, output) in outputs.iter().take(count).enumerate() {
        if single && Some(index) != input {
            encoded.extend_from_slice(&(-1_i64).to_le_bytes());
            encoded.push(0);
        } else {
            output.write(&mut encoded).map_err(|_| Error::Encoding)?;
        }
    }
    encoded.extend_from_slice(&tx.lock_time().to_le_bytes());
    if version >= 2 {
        let bundle = tx.sprout_bundle();
        CompactSize::write(&mut encoded, bundle.map_or(0, |b| b.joinsplits.len()))
            .map_err(|_| Error::Encoding)?;
        if let Some(bundle) = bundle {
            for js in &bundle.joinsplits {
                js.write(&mut encoded).map_err(|_| Error::Encoding)?;
            }
            encoded.extend_from_slice(&bundle.joinsplit_pubkey);
            encoded.extend_from_slice(&[0; 64]);
        }
    }
    encoded.extend_from_slice(&hash_type.to_le_bytes());
    Ok(Sha256::digest(Sha256::digest(encoded)).into())
}

fn key(outpoint: &OutPoint) -> OutputKey {
    (*outpoint.hash(), outpoint.n())
}
fn money_add(left: u64, right: u64) -> Result<u64> {
    left.checked_add(right)
        .filter(|sum| *sum <= MAX_MONEY)
        .ok_or(Error::Value)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use secp256k1::{Message, Secp256k1, SecretKey};
    use transparent::bundle::{Authorized, Bundle, TxIn};
    use zcash_protocol::consensus::BlockHeight;

    fn transaction(
        version: TxVersion,
        inputs: Vec<TxIn<Authorized>>,
        outputs: Vec<TxOut>,
        expiry: u32,
        lock: u32,
    ) -> Transaction {
        TransactionData::<zcash_primitives::transaction::Authorized>::from_parts(
            version,
            match version {
                TxVersion::Sprout(_) => BranchId::Sprout,
                TxVersion::V3 => BranchId::Overwinter,
                _ => BranchId::Sapling,
            },
            lock,
            BlockHeight::from_u32(expiry),
            Some(Bundle {
                vin: inputs,
                vout: outputs,
                authorization: Authorized,
            }),
            None,
            None,
            None,
        )
        .freeze()
        .unwrap()
    }
    fn output(value: u64, script: Vec<u8>) -> TxOut {
        TxOut::new(
            Zatoshis::from_u64(value).unwrap(),
            Script(script::Code(script)),
        )
    }
    fn seeded(coinbase: bool) -> (CandidateLedger, OutPoint) {
        let outpoint = OutPoint::new([7; 32], 0);
        let mut ledger = CandidateLedger::default();
        ledger
            .insert_output(
                key(&outpoint),
                UnspentOutput {
                    output: output(100, vec![0x51]),
                    height: 10,
                    coinbase,
                },
            )
            .unwrap();
        (ledger, outpoint)
    }
    #[test]
    fn legacy_hash_matches_zcashd_vectors_including_unknown_hash_bits() {
        let vectors: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../../tests/fixtures/legacy-sighash.json"))
                .unwrap();
        for vector in vectors {
            let raw = hex::decode(vector[0].as_str().unwrap()).unwrap();
            let tx = Transaction::read(raw.as_slice(), BranchId::Sprout).unwrap();
            let TxVersion::Sprout(version) = tx.version() else {
                panic!("legacy fixture version")
            };
            let mut expected: [u8; 32] = hex::decode(vector[5].as_str().unwrap())
                .unwrap()
                .try_into()
                .unwrap();
            expected.reverse();
            assert_eq!(
                legacy_hash(
                    &tx,
                    version,
                    Some(vector[2].as_u64().unwrap() as usize),
                    &script::Code(hex::decode(vector[1].as_str().unwrap()).unwrap()),
                    vector[3].as_i64().unwrap() as u32
                )
                .unwrap(),
                expected
            );
        }
    }
    #[test]
    fn transparent_spends_conserve_value_and_cannot_spend_twice() {
        let (mut ledger, previous) = seeded(false);
        let tx = transaction(
            TxVersion::Sprout(1),
            vec![TxIn::from_parts(
                previous.clone(),
                Script::default(),
                u32::MAX,
            )],
            vec![output(90, vec![0x51])],
            0,
            0,
        );
        assert_eq!(ledger.apply(&tx, 11, false).unwrap(), (10, 0));
        assert!(ledger.output(&previous).unwrap().is_none());
        assert_eq!(ledger.apply(&tx, 11, false), Err(Error::Input));
        assert_eq!(ledger.len(), 1);
    }
    #[test]
    fn duplicate_inputs_and_negative_fees_are_rejected_before_consumption() {
        let (mut ledger, previous) = seeded(false);
        let input = || TxIn::from_parts(previous.clone(), Script::default(), u32::MAX);
        let duplicate = transaction(
            TxVersion::Sprout(1),
            vec![input(), input()],
            vec![output(1, vec![0x51])],
            0,
            0,
        );
        assert_eq!(ledger.apply(&duplicate, 11, false), Err(Error::Input));
        let inflation = transaction(
            TxVersion::Sprout(1),
            vec![input()],
            vec![output(101, vec![0x51])],
            0,
            0,
        );
        assert_eq!(ledger.apply(&inflation, 11, false), Err(Error::Value));
        assert!(ledger.output(&previous).unwrap().is_some());
    }
    #[test]
    fn coinbase_spends_require_maturity_and_shielded_outputs() {
        let (mut ledger, previous) = seeded(true);
        let tx = transaction(
            TxVersion::Sprout(1),
            vec![TxIn::from_parts(previous, Script::default(), u32::MAX)],
            vec![output(90, vec![0x51])],
            0,
            0,
        );
        assert_eq!(ledger.apply(&tx, 109, false), Err(Error::CoinbaseSpend));
        assert_eq!(ledger.apply(&tx, 110, false), Err(Error::CoinbaseSpend));
    }
    #[test]
    fn script_signatures_bind_legacy_outputs() {
        let secret = SecretKey::from_slice(&[9; 32]).unwrap();
        let secp = Secp256k1::new();
        let public = secret.public_key(&secp).serialize();
        let mut script = vec![33];
        script.extend_from_slice(&public);
        script.push(0xac);
        let previous = output(100, script.clone());
        let outpoint = OutPoint::new([7; 32], 0);
        let make = |value, unlocking| {
            transaction(
                TxVersion::Sprout(1),
                vec![TxIn::from_parts(
                    outpoint.clone(),
                    Script(script::Code(unlocking)),
                    u32::MAX,
                )],
                vec![output(value, vec![0x51])],
                0,
                0,
            )
        };
        let unsigned = make(90, vec![]);
        let hash = input_signature_hash(
            &unsigned,
            std::slice::from_ref(&previous),
            Some(0),
            &script::Code(script),
            1,
        )
        .unwrap();
        let signature = secp
            .sign_ecdsa(&Message::from_digest(hash), &secret)
            .serialize_der();
        let mut unlocking = vec![(signature.len() + 1) as u8];
        unlocking.extend_from_slice(&signature);
        unlocking.push(1);
        let mut ledger = CandidateLedger::default();
        ledger
            .insert_output(
                key(&outpoint),
                UnspentOutput {
                    output: previous,
                    height: 1,
                    coinbase: false,
                },
            )
            .unwrap();
        assert_eq!(
            ledger
                .clone()
                .apply(&make(91, unlocking.clone()), 11, false),
            Err(Error::Script)
        );
        assert_eq!(
            ledger.apply(&make(90, unlocking), 11, false).unwrap(),
            (10, 0)
        );
    }
    #[test]
    fn expiry_is_inclusive_and_lock_time_is_strict() {
        let (_, outpoint) = seeded(false);
        let tx = transaction(
            TxVersion::V3,
            vec![TxIn::from_parts(outpoint, Script::default(), 0)],
            vec![output(1, vec![0x51])],
            20,
            10,
        );
        assert_eq!(
            check_context(&tx, BranchId::Overwinter, 10, 100, false),
            Err(Error::LockTime)
        );
        assert!(check_context(&tx, BranchId::Overwinter, 20, 100, false).is_ok());
        assert_eq!(
            check_context(&tx, BranchId::Overwinter, 21, 100, false),
            Err(Error::Expiry)
        );
        assert_eq!(
            check_context(&tx, BranchId::Sapling, 20, 100, false),
            Err(Error::Version)
        );
    }
    #[test]
    fn real_first_block_does_not_make_genesis_outputs_spendable() {
        let first = ParsedBlock::parse(
            &hex::decode(include_str!("../../../tests/fixtures/block-1.txt").trim()).unwrap(),
            &zcash_protocol::consensus::MAIN_NETWORK,
        )
        .unwrap();
        let mut ledger = CandidateLedger::default();
        let result = ledger.append(&first).unwrap();
        assert_eq!(result.fees, 0);
        assert!(result.coinbase_value > 0);
        let before = ledger.len();
        assert!(ledger.append(&first).is_err());
        assert_eq!(ledger.len(), before);
    }
}
