use crate::{Error, Result};
use incrementalmerkletree::{frontier::CommitmentTree, witness::IncrementalWitness};
use orchard::{
    Note,
    keys::{FullViewingKey, PreparedIncomingViewingKey, Scope},
    note::{ExtractedNoteCommitment, NoteVersion, RandomSeed, Rho},
    note_encryption::IronwoodDomain,
    tree::{MerkleHashOrchard, MerklePath},
    value::NoteValue,
};
use zcash_note_encryption::try_note_decryption;
use zcash_primitives::merkle_tree::read_commitment_tree;
use zcash_protocol::consensus::TEST_NETWORK;
use zutti_protocol::{
    Digest, Secret32,
    encoding::{Canonical, Reader, Writer},
    order::OrderTerms,
};

pub struct SourceNote {
    pub note: Note,
    pub path: MerklePath,
    pub txid: Digest,
    pub action: u32,
    pub received_height: u32,
}

impl core::fmt::Debug for SourceNote {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SourceNote([REDACTED])")
    }
}

impl Canonical for SourceNote {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        if self.note.version() != NoteVersion::V3 {
            return Err(zutti_protocol::Error::Invalid("Ironwood note version"));
        }
        writer.fixed(&self.note.recipient().to_raw_address_bytes());
        writer.u64(self.note.value().inner());
        writer.fixed(&self.note.rho().to_bytes());
        writer.fixed(self.note.rseed().as_bytes());
        writer.u32(self.path.position());
        for node in self.path.auth_path() {
            writer.fixed(&node.to_bytes());
        }
        writer.fixed(&self.txid.0);
        writer.u32(self.action);
        writer.u32(self.received_height);
        Ok(())
    }

    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        let error = || zutti_protocol::Error::Invalid("Ironwood note archive");
        let recipient = orchard::Address::from_raw_address_bytes(&reader.fixed()?)
            .into_option()
            .ok_or_else(error)?;
        let value = NoteValue::from_raw(reader.u64()?);
        let rho = Rho::from_bytes(&reader.fixed()?)
            .into_option()
            .ok_or_else(error)?;
        let rseed = RandomSeed::from_bytes(reader.fixed()?, &rho)
            .into_option()
            .ok_or_else(error)?;
        let note = Note::from_parts(recipient, value, rho, rseed, NoteVersion::V3)
            .into_option()
            .ok_or_else(error)?;
        let position = reader.u32()?;
        let mut nodes =
            [MerkleHashOrchard::from_cmx(&ExtractedNoteCommitment::from(note.commitment())); 32];
        for node in &mut nodes {
            *node = MerkleHashOrchard::from_bytes(&reader.fixed()?)
                .into_option()
                .ok_or_else(error)?;
        }
        Ok(Self {
            note,
            path: MerklePath::from_parts(position, nodes),
            txid: Digest::nonzero(reader.fixed()?)?,
            action: reader.u32()?,
            received_height: reader.u32()?,
        })
    }
}

pub struct SourceScan {
    pub height: u32,
    pub block: Digest,
    pub anchor: [u8; 32],
    pub notes: Vec<SourceNote>,
}

impl core::fmt::Debug for SourceScan {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SourceScan")
            .field("height", &self.height)
            .field("notes", &self.notes.len())
            .finish_non_exhaustive()
    }
}

impl Canonical for SourceScan {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        if self.notes.len() > 64 {
            return Err(zutti_protocol::Error::Length);
        }
        writer.fixed(b"ZUTTISCN");
        writer.u16(1);
        writer.u32(self.height);
        writer.fixed(&self.block.0);
        writer.fixed(&self.anchor);
        writer.u32(self.notes.len() as u32);
        for note in &self.notes {
            note.encode(writer)?;
        }
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTISCN" || reader.u16()? != 1 {
            return Err(zutti_protocol::Error::Version);
        }
        let height = reader.u32()?;
        let block = Digest::nonzero(reader.fixed()?)?;
        let anchor = reader.fixed()?;
        let count = reader.u32()?;
        if count > 64 {
            return Err(zutti_protocol::Error::Length);
        }
        let mut notes = Vec::new();
        for _ in 0..count {
            let note = SourceNote::decode(reader)?;
            if note.received_height > height
                || note
                    .path
                    .root(ExtractedNoteCommitment::from(note.note.commitment()))
                    .to_bytes()
                    != anchor
            {
                return Err(zutti_protocol::Error::Invalid("source note membership"));
            }
            notes.push(note);
        }
        Ok(Self {
            height,
            block,
            anchor,
            notes,
        })
    }
}

impl SourceScan {
    pub fn prepare_payment(
        &self,
        viewing: &FullViewingKey,
        terms: &OrderTerms,
        randomness: &Secret32,
    ) -> Result<Vec<u8>> {
        use pczt::roles::{creator::Creator, io_finalizer::IoFinalizer};
        use rand::{rand_core::UnwrapErr, rngs::SysRng};
        use zcash_primitives::transaction::{
            builder::{BuildConfig, Builder, BundlePadding},
            fees::zip317,
        };
        use zcash_protocol::{consensus::BlockHeight, memo::MemoBytes, value::Zatoshis};
        if terms.domain().source_genesis() != zutti_protocol::network::ZCASH_TESTNET_GENESIS
            || terms.window().anchor_height() != self.height
            || terms.window().anchor_hash() != self.block
        {
            return Err(Error::Payment);
        }
        let amount = u64::try_from(terms.zec().units()).map_err(|_| Error::Payment)?;
        let minimum = u64::from(zip317::MINIMUM_FEE);
        let required = amount.checked_add(minimum).ok_or(Error::Payment)?;
        let coin = self
            .notes
            .iter()
            .find(|coin| coin.note.value().inner() >= required)
            .ok_or(Error::SourceState("insufficient Ironwood balance"))?;
        if viewing.scope_for_address(&coin.note.recipient()).is_none()
            || coin.note.version() != NoteVersion::V3
            || coin
                .path
                .root(ExtractedNoteCommitment::from(coin.note.commitment()))
                .to_bytes()
                != self.anchor
        {
            return Err(Error::SourceState("payment note membership"));
        }
        let recipient = orchard::Address::from_raw_address_bytes(terms.receiver())
            .into_option()
            .ok_or(Error::Payment)?;
        let height = self.height.checked_add(1).ok_or(Error::Payment)?;
        let mut builder = Builder::new(
            TEST_NETWORK,
            BlockHeight::from_u32(height),
            BuildConfig::Standard {
                sapling_anchor: None,
                orchard_anchor: None,
                ironwood_anchor: Some(
                    coin.path
                        .root(ExtractedNoteCommitment::from(coin.note.commitment())),
                ),
                orchard_padding: BundlePadding::DEFAULT,
                ironwood_padding: BundlePadding::DEFAULT,
            },
        )
        .with_expiry_height(terms.window().expiry().into());
        builder
            .add_ironwood_spend::<zip317::FeeRule>(viewing.clone(), coin.note, coin.path.clone())
            .map_err(|_| Error::Payment)?;
        builder
            .add_ironwood_output::<zip317::FeeRule>(
                Some(viewing.to_ovk(Scope::External)),
                recipient,
                Zatoshis::from_u64(amount).map_err(|_| Error::Payment)?,
                MemoBytes::from_bytes(&terms.memo(terms.commitment(randomness)?)?)
                    .map_err(|_| Error::Payment)?,
            )
            .map_err(|_| Error::Payment)?;
        let fee = builder
            .get_fee(&zip317::FeeRule::standard())
            .map_err(|_| Error::Payment)?;
        if u128::from(u64::from(fee)) > terms.source_fee_cap().units() {
            return Err(Error::Payment);
        }
        let change = coin
            .note
            .value()
            .inner()
            .checked_sub(amount)
            .and_then(|balance| balance.checked_sub(u64::from(fee)))
            .ok_or(Error::Payment)?;
        if change > 0 {
            builder
                .add_ironwood_output::<zip317::FeeRule>(
                    Some(viewing.to_ovk(Scope::Internal)),
                    viewing.address_at(0_u32, Scope::Internal),
                    Zatoshis::from_u64(change).map_err(|_| Error::Payment)?,
                    MemoBytes::empty(),
                )
                .map_err(|_| Error::Payment)?;
        }
        if builder
            .get_fee(&zip317::FeeRule::standard())
            .map_err(|_| Error::Payment)?
            != fee
        {
            return Err(Error::Payment);
        }
        let parts = builder
            .build_for_pczt(UnwrapErr(SysRng), &zip317::FeeRule::standard())
            .map_err(|_| Error::Payment)?;
        let pczt =
            IoFinalizer::new(Creator::build_from_parts(parts.pczt_parts).ok_or(Error::Pczt)?)
                .finalize_io(UnwrapErr(SysRng))
                .map_err(|_| Error::Pczt)?;
        let encoded = pczt.serialize().map_err(|_| Error::Pczt)?;
        crate::prepare::inspect_ironwood(&encoded, terms, randomness)?;
        Ok(encoded)
    }
}

#[cfg(feature = "proving")]
pub fn prove_payment(bytes: &[u8], terms: &OrderTerms, randomness: &Secret32) -> Result<Vec<u8>> {
    use orchard::circuit::{OrchardCircuitVersion, ProvingKey};
    use pczt::{Pczt, roles::prover::Prover};
    use rand::{rand_core::UnwrapErr, rngs::SysRng};
    static KEY: std::sync::OnceLock<ProvingKey> = std::sync::OnceLock::new();
    let before = crate::prepare::inspect_ironwood(bytes, terms, randomness)?;
    let pczt = Pczt::parse(bytes).map_err(|_| Error::Pczt)?;
    let key = KEY.get_or_init(|| ProvingKey::build(OrchardCircuitVersion::PostNu6_3));
    let proven = Prover::new(pczt)
        .create_ironwood_proof(UnwrapErr(SysRng), key)
        .map_err(|_| Error::Transaction)?
        .finish()
        .serialize()
        .map_err(|_| Error::Pczt)?;
    if crate::prepare::inspect_ironwood(&proven, terms, randomness)?.txid() != before.txid() {
        return Err(Error::Transaction);
    }
    Ok(proven)
}

pub fn scan_ironwood(
    viewing: &FullViewingKey,
    previous_height: u32,
    previous_hash: Digest,
    tree_bytes: &[u8],
    blocks: &[Vec<u8>],
    expected_anchor: [u8; 32],
) -> Result<SourceScan> {
    if blocks.is_empty()
        || blocks.len() > 4096
        || tree_bytes.len() > 4096
        || blocks
            .iter()
            .map(Vec::len)
            .try_fold(0_usize, usize::checked_add)
            .is_none_or(|bytes| bytes > 16_777_216)
    {
        return Err(Error::SourceState("scan bounds"));
    }
    let mut input = tree_bytes;
    let mut tree: CommitmentTree<MerkleHashOrchard, 32> = read_commitment_tree(&mut input)?;
    if !input.is_empty() {
        return Err(Error::SourceState("trailing commitment tree data"));
    }
    let incoming = [Scope::External, Scope::Internal]
        .map(|scope| PreparedIncomingViewingKey::new(&viewing.to_ivk(scope)));
    let mut found: Vec<(SourceNote, IncrementalWitness<MerkleHashOrchard, 32>)> = Vec::new();
    let mut spent = std::collections::BTreeSet::new();
    let mut height = previous_height;
    let mut parent = previous_hash;
    for raw in blocks {
        let block = zutti_source::block::ParsedBlock::parse(raw, &TEST_NETWORK)
            .map_err(|_| Error::SourceState("block encoding or transaction commitments"))?;
        height = height.checked_add(1).ok_or(Error::Archive)?;
        if block.claimed_height() != height || block.parent() != parent {
            return Err(Error::SourceState("block height or parent hash"));
        }
        for transaction in block.transactions() {
            if let Some(bundle) = transaction.ironwood_bundle() {
                for (index, action) in bundle.actions().iter().enumerate() {
                    spent.insert(action.nullifier().to_bytes());
                    let cm = MerkleHashOrchard::from_cmx(action.cmx());
                    for (_, witness) in &mut found {
                        witness.append(cm).map_err(|_| Error::Archive)?;
                    }
                    tree.append(cm).map_err(|_| Error::Archive)?;
                    let domain = IronwoodDomain::for_action(action);
                    let decrypted = incoming
                        .iter()
                        .find_map(|key| try_note_decryption(&domain, key, action));
                    if let Some((note, _, _)) = decrypted {
                        if found.len() >= 64 {
                            return Err(Error::Archive);
                        }
                        let witness =
                            IncrementalWitness::from_tree(tree.clone()).ok_or(Error::Archive)?;
                        let path = witness.path().ok_or(Error::Archive)?.into();
                        found.push((
                            SourceNote {
                                note,
                                path,
                                txid: Digest(*transaction.txid().as_ref()),
                                action: u32::try_from(index).map_err(|_| Error::Archive)?,
                                received_height: height,
                            },
                            witness,
                        ));
                    }
                }
            }
        }
        parent = block.hash();
    }
    if tree.root().to_bytes() != expected_anchor {
        return Err(Error::SourceState(
            "computed Ironwood root differs from the supplied state",
        ));
    }
    let mut notes = Vec::new();
    for (mut owned, witness) in found {
        if spent.contains(&owned.note.nullifier(viewing).to_bytes()) {
            continue;
        }
        owned.path = witness.path().ok_or(Error::Archive)?.into();
        if owned
            .path
            .root(ExtractedNoteCommitment::from(owned.note.commitment()))
            .to_bytes()
            != expected_anchor
        {
            return Err(Error::Archive);
        }
        notes.push(owned);
    }
    Ok(SourceScan {
        height,
        block: parent,
        anchor: expected_anchor,
        notes,
    })
}
