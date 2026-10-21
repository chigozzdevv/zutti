use crate::{Error, Result, backup, prepare::MAX_PCZT, swap::SwapRecord};
use pczt::{
    Pczt,
    roles::{
        redactor::Redactor,
        signer::{Signer, SpendAuthSignature, extract_orchard_spend_auth_signatures},
        tx_extractor::TransactionExtractor,
    },
};
use std::{
    fs,
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;
use zutti_protocol::{
    Digest, Secret32,
    commitment::hash,
    encoding::{Canonical, Reader, Writer},
};

const MAX_PAYMENT_ARCHIVE: usize = MAX_PCZT + crate::swap::MAX_SWAP_RECORD + 256;
const STAGES: [&str; 5] = [
    "signing.backup",
    "signed.backup",
    "transaction.backup",
    "attempt.backup",
    "acknowledged.backup",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaymentState {
    Prepared,
    Signing,
    Signed,
    Ready,
    Attempted,
    Acknowledged,
}

pub struct PaymentJournal {
    directory: PathBuf,
    key: Secret32,
    record: SwapRecord,
    prepared: Zeroizing<Vec<u8>>,
    binding: Digest,
    txid: Digest,
}
impl core::fmt::Debug for PaymentJournal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("PaymentJournal([REDACTED])")
    }
}
impl PaymentJournal {
    pub fn create(
        directory: &Path,
        key: Secret32,
        record: SwapRecord,
        pczt: Vec<u8>,
    ) -> Result<Self> {
        let pczt = Zeroizing::new(pczt);
        let payment = record.inspect_payment(&pczt)?;
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(directory)?;
        private_directory(directory)?;
        let mut writer = Writer::default();
        writer.fixed(b"ZUTTIPAY");
        writer.u16(1);
        let record_bytes = Zeroizing::new(record.to_bytes()?);
        writer.bytes(&record_bytes)?;
        writer.bytes(&pczt)?;
        let plaintext = Zeroizing::new(writer.finish());
        let archive = backup::seal(record.domain(), &plaintext, &key)?;
        backup::persist_new(&directory.join("prepared.backup"), &archive)?;
        fs::File::open(directory.parent().unwrap_or(Path::new(".")))?.sync_all()?;
        Self::assemble(directory, key, record, pczt, payment.txid())
    }
    pub fn open(directory: &Path, key: Secret32, domain: Digest, swap: Digest) -> Result<Self> {
        private_directory(directory)?;
        let archive =
            backup::read_private(&directory.join("prepared.backup"), MAX_PAYMENT_ARCHIVE)?;
        let plaintext = backup::open(domain, &archive, &key)?;
        let mut reader = Reader::new(&plaintext);
        if reader.fixed::<8>()? != *b"ZUTTIPAY" || reader.u16()? != 1 {
            return Err(Error::Archive);
        }
        let record = SwapRecord::from_bytes(reader.bytes(crate::swap::MAX_SWAP_RECORD)?)?;
        let pczt = Zeroizing::new(reader.bytes(MAX_PCZT)?.to_vec());
        reader.finish()?;
        if record.domain() != domain || record.id()? != swap {
            return Err(Error::Archive);
        }
        let payment = record.inspect_payment(&pczt)?;
        let journal = Self::assemble(directory, key, record, pczt, payment.txid())?;
        journal.state()?;
        Ok(journal)
    }
    fn assemble(
        directory: &Path,
        key: Secret32,
        record: SwapRecord,
        prepared: Zeroizing<Vec<u8>>,
        txid: Digest,
    ) -> Result<Self> {
        let binding = hash(b"payment-journal", &[&record.id()?.0, &prepared]);
        Ok(Self {
            directory: directory.to_path_buf(),
            key,
            record,
            prepared,
            binding,
            txid,
        })
    }
    pub fn record(&self) -> &SwapRecord {
        &self.record
    }
    pub fn unsigned_pczt(&self) -> &[u8] {
        &self.prepared
    }
    pub const fn txid(&self) -> Digest {
        self.txid
    }
    pub fn state(&self) -> Result<PaymentState> {
        private_directory(&self.directory)?;
        let mut count = 0;
        let mut missing = false;
        for (index, name) in STAGES.iter().enumerate() {
            match fs::symlink_metadata(self.directory.join(name)) {
                Ok(_) => {
                    if missing {
                        return Err(Error::PaymentState);
                    }
                    let payload = self.read_stage(index)?;
                    match index {
                        0 | 3 if !payload.is_empty() => return Err(Error::Archive),
                        1 => {
                            self.validate_signed(&payload)?;
                        }
                        2 if payload.is_empty() || payload.len() > MAX_PCZT => {
                            return Err(Error::Archive);
                        }
                        4 if payload.as_slice() != self.txid.0 => return Err(Error::Archive),
                        _ => {}
                    }
                    count += 1;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => missing = true,
                Err(error) => return Err(error.into()),
            }
        }
        Ok([
            PaymentState::Prepared,
            PaymentState::Signing,
            PaymentState::Signed,
            PaymentState::Ready,
            PaymentState::Attempted,
            PaymentState::Acknowledged,
        ][count])
    }
    pub fn begin_signing(&self) -> Result<()> {
        if self.state()? != PaymentState::Prepared {
            return Err(Error::PaymentState);
        }
        self.save_stage(0, &[])
    }
    pub fn record_signatures(&self, signatures: &[SpendAuthSignature]) -> Result<()> {
        if self.state()? != PaymentState::Signing {
            return Err(Error::PaymentState);
        }
        let signed = self.apply_signatures(signatures)?;
        self.save_stage(1, &signed)
    }
    fn apply_signatures(&self, signatures: &[SpendAuthSignature]) -> Result<Zeroizing<Vec<u8>>> {
        let pczt = Pczt::parse(&self.prepared).map_err(|_| Error::Pczt)?;
        let required: Vec<_> = pczt
            .ironwood()
            .actions()
            .iter()
            .enumerate()
            .filter(|(_, action)| action.spend().spend_auth_sig().is_none())
            .map(|(index, _)| index)
            .collect();
        if signatures.len() != required.len() {
            return Err(Error::Signatures);
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut signer = Signer::new(pczt).map_err(|_| Error::Signatures)?;
        for signature in signatures {
            if signature.value_pool() != orchard::ValuePool::Ironwood
                || !required.contains(&signature.action_index())
                || !seen.insert(signature.action_index())
            {
                return Err(Error::Signatures);
            }
            signer
                .apply_orchard_spend_auth_signature(signature)
                .map_err(|_| Error::Signatures)?;
        }
        let pczt = signer.finish();
        if pczt
            .ironwood()
            .actions()
            .iter()
            .any(|action| action.spend().spend_auth_sig().is_none())
        {
            return Err(Error::Signatures);
        }
        let signed = Zeroizing::new(pczt.serialize().map_err(|_| Error::Pczt)?);
        if signed.len() > MAX_PCZT {
            return Err(Error::Pczt);
        }
        Ok(signed)
    }
    fn validate_signed(&self, signed: &[u8]) -> Result<()> {
        let pczt = Pczt::parse(signed).map_err(|_| Error::Pczt)?;
        let unsigned = Pczt::parse(&self.prepared).map_err(|_| Error::Pczt)?;
        let signatures: Vec<_> = extract_orchard_spend_auth_signatures(&pczt)
            .into_iter()
            .filter(|s| {
                s.value_pool() == orchard::ValuePool::Ironwood
                    && unsigned
                        .ironwood()
                        .actions()
                        .get(s.action_index())
                        .is_some_and(|a| a.spend().spend_auth_sig().is_none())
            })
            .collect();
        if self.apply_signatures(&signatures)?.as_slice() != signed {
            return Err(Error::Signatures);
        }
        Ok(())
    }
    pub fn finalize(&self) -> Result<Digest> {
        match self.state()? {
            PaymentState::Ready | PaymentState::Attempted | PaymentState::Acknowledged => {
                return Ok(self.txid);
            }
            PaymentState::Signed => {}
            _ => return Err(Error::PaymentState),
        }
        let signed = self.read_stage(1)?;
        self.extract_transaction(&signed)
    }
    pub fn finalize_with_proof(&self, proven_pczt: Vec<u8>) -> Result<Digest> {
        let proven = Zeroizing::new(proven_pczt);
        if self.state()? != PaymentState::Signed || proven.len() > MAX_PCZT {
            return Err(Error::PaymentState);
        }
        let signed = self.read_stage(1)?;
        let strip_proof = |bytes: &[u8]| -> Result<Zeroizing<Vec<u8>>> {
            let pczt = Pczt::parse(bytes).map_err(|_| Error::Pczt)?;
            Ok(Zeroizing::new(
                Redactor::new(pczt)
                    .redact_ironwood_with(|mut bundle| bundle.clear_zkproof())
                    .finish()
                    .serialize()
                    .map_err(|_| Error::Pczt)?,
            ))
        };
        if strip_proof(&signed)? != strip_proof(&proven)? {
            return Err(Error::Transaction);
        }
        self.extract_transaction(&proven)
    }
    fn extract_transaction(&self, signed: &[u8]) -> Result<Digest> {
        let pczt = Pczt::parse(signed).map_err(|_| Error::Pczt)?;
        if pczt.ironwood().zkproof().is_none() {
            return Err(Error::Transaction);
        }
        let transaction = TransactionExtractor::new(pczt)
            .extract(rand::rng())
            .map_err(|_| Error::Transaction)?;
        if transaction.txid().as_ref() != &self.txid.0 {
            return Err(Error::Transaction);
        }
        let mut bytes = Zeroizing::new(Vec::new());
        transaction
            .write(&mut *bytes)
            .map_err(|_| Error::Transaction)?;
        if bytes.is_empty() || bytes.len() > MAX_PCZT {
            return Err(Error::Transaction);
        }
        self.save_stage(2, &bytes)?;
        Ok(self.txid)
    }
    pub fn submit<F>(&self, transport: F) -> Result<Digest>
    where
        F: FnOnce(&[u8]) -> Result<Digest>,
    {
        match self.state()? {
            PaymentState::Ready => {
                self.save_stage(3, &[])?;
            }
            PaymentState::Attempted => {}
            PaymentState::Acknowledged => return Ok(self.txid),
            _ => return Err(Error::PaymentState),
        }
        let bytes = self.read_stage(2)?;
        if transport(&bytes)? != self.txid {
            return Err(Error::Broadcast);
        }
        match self.save_stage(4, &self.txid.0) {
            Ok(()) => {}
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if self.state()? != PaymentState::Acknowledged {
                    return Err(Error::PaymentState);
                }
            }
            Err(error) => return Err(error),
        }
        Ok(self.txid)
    }
    fn save_stage(&self, stage: usize, payload: &[u8]) -> Result<()> {
        private_directory(&self.directory)?;
        let mut writer = Writer::default();
        writer.fixed(b"ZUTTIPST");
        writer.u16(1);
        writer.u8(stage as u8);
        writer.fixed(&self.binding.0);
        writer.bytes(payload)?;
        let plaintext = Zeroizing::new(writer.finish());
        let archive = backup::seal(self.record.domain(), &plaintext, &self.key)?;
        backup::persist_new(&self.directory.join(STAGES[stage]), &archive)
    }
    fn read_stage(&self, stage: usize) -> Result<Zeroizing<Vec<u8>>> {
        let archive = backup::read_private(&self.directory.join(STAGES[stage]), MAX_PCZT + 256)?;
        let plaintext = backup::open(self.record.domain(), &archive, &self.key)?;
        let mut reader = Reader::new(&plaintext);
        if reader.fixed::<8>()? != *b"ZUTTIPST"
            || reader.u16()? != 1
            || reader.u8()? != stage as u8
            || Digest(reader.fixed()?) != self.binding
        {
            return Err(Error::Archive);
        }
        let payload = Zeroizing::new(reader.bytes(MAX_PCZT)?.to_vec());
        reader.finish()?;
        Ok(payload)
    }
}
fn private_directory(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(Error::Permissions);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(Error::Permissions);
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../../tests/support/payment.rs"]
mod payment_fixture;
#[cfg(test)]
#[path = "../../../tests/support/payment_swap.rs"]
mod payment_swap;
#[cfg(test)]
#[path = "../../../tests/support/mod.rs"]
mod support;
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use orchard::keys::SpendAuthorizingKey;
    use rand::{SeedableRng, rngs::StdRng};

    #[test]
    fn transport_retries_exact_bytes_after_a_lost_response_and_restart() {
        let (fixture, record) = payment_swap::fixture();
        let domain = record.domain();
        let swap = record.id().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("payment");
        let journal = PaymentJournal::create(
            &path,
            support::secret(200),
            record,
            fixture.pczt.clone().serialize().unwrap(),
        )
        .unwrap();
        let mut signer = Signer::new(fixture.pczt).unwrap();
        signer
            .sign_ironwood(
                StdRng::from_seed([44; 32]),
                fixture.spend,
                &SpendAuthorizingKey::from(&fixture.key),
            )
            .unwrap();
        let signatures: Vec<_> = extract_orchard_spend_auth_signatures(&signer.finish())
            .into_iter()
            .filter(|s| s.action_index() == fixture.spend)
            .collect();
        journal.begin_signing().unwrap();
        journal.record_signatures(&signatures).unwrap();
        let bytes = b"generated transport fixture, not a valid transaction";
        journal.save_stage(2, bytes).unwrap();
        assert!(
            journal
                .submit(|sent| {
                    assert_eq!(sent, bytes);
                    Err(Error::Io(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "lost response",
                    )))
                })
                .is_err()
        );
        assert_eq!(journal.state().unwrap(), PaymentState::Attempted);
        drop(journal);
        let journal = PaymentJournal::open(&path, support::secret(200), domain, swap).unwrap();
        assert!(matches!(
            journal.submit(|sent| {
                assert_eq!(sent, bytes);
                Ok(Digest([9; 32]))
            }),
            Err(Error::Broadcast)
        ));
        assert_eq!(journal.state().unwrap(), PaymentState::Attempted);
        assert_eq!(
            journal
                .submit(|sent| {
                    assert_eq!(sent, bytes);
                    Ok(journal.txid())
                })
                .unwrap(),
            journal.txid()
        );
        assert_eq!(journal.state().unwrap(), PaymentState::Acknowledged);
        assert_eq!(
            journal
                .submit(|_| panic!("acknowledged payments do not resubmit"))
                .unwrap(),
            journal.txid()
        );
    }

    #[test]
    fn interrupted_stage_sequence_does_not_fall_back_to_prepared() {
        let (fixture, record) = payment_swap::fixture();
        let directory = tempfile::tempdir().unwrap();
        let journal = PaymentJournal::create(
            &directory.path().join("payment"),
            support::secret(200),
            record,
            fixture.pczt.serialize().unwrap(),
        )
        .unwrap();
        journal.save_stage(2, b"orphaned fixture").unwrap();
        assert!(matches!(journal.state(), Err(Error::PaymentState)));
        assert!(journal.begin_signing().is_err());
    }
}
