use crate::{Error, Result, backup, private_note::NoteKeys};
use orchard::keys::{FullViewingKey, Scope, SpendingKey};
use std::{fs, path::Path};
use zcash_address::unified::{Address, Encoding, Receiver, Uitem};
use zcash_protocol::{address::Revision, consensus::NetworkType};
use zeroize::Zeroizing;
use zutti_protocol::{
    Digest, Secret32,
    commitment::hash,
    encoding::{Reader, Writer},
    network::ZCASH_TESTNET_GENESIS,
    private_note::NoteRecipient,
};

pub struct WalletAccount {
    source: SpendingKey,
    notes: NoteKeys,
}

impl core::fmt::Debug for WalletAccount {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("WalletAccount([REDACTED])")
    }
}

fn domain() -> Digest {
    hash(
        b"testnet-wallet",
        &[&ZCASH_TESTNET_GENESIS.0, &11155111_u64.to_le_bytes()],
    )
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

impl WalletAccount {
    pub fn create(directory: &Path) -> Result<Self> {
        let mut source = Secret32::random()?;
        let mut source_key = None;
        for _ in 0..16 {
            if let Some(key) =
                Option::<SpendingKey>::from(SpendingKey::from_bytes(*source.expose()))
            {
                source_key = Some(key);
                break;
            }
            source = Secret32::random()?;
        }
        let source_key = source_key.ok_or(zutti_protocol::Error::Entropy)?;
        let spending = Secret32::random()?;
        let viewing = Secret32::random()?;
        let nullifier = Secret32::random()?;
        let notes = NoteKeys::from_secrets(
            Secret32::from_bytes(*spending.expose())?,
            Secret32::from_bytes(*viewing.expose())?,
            Secret32::from_bytes(*nullifier.expose())?,
        )?;
        let mut writer = Writer::default();
        writer.fixed(b"ZUTTIACC");
        writer.u16(1);
        for value in [&source, &spending, &viewing, &nullifier] {
            writer.fixed(value.expose());
        }
        let plaintext = Zeroizing::new(writer.finish());
        let key = Secret32::random()?;
        let encrypted = backup::seal(domain(), &plaintext, &key)?;
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(directory)?;
        private_directory(directory)?;
        backup::persist_new(&directory.join("account.key"), key.expose())?;
        backup::persist_new(&directory.join("account.backup"), &encrypted)?;
        let parent = directory.parent().unwrap_or(Path::new("."));
        fs::File::open(parent)?.sync_all()?;
        Ok(Self {
            source: source_key,
            notes,
        })
    }

    pub fn open(directory: &Path) -> Result<Self> {
        private_directory(directory)?;
        let key = backup::read_private(&directory.join("account.key"), 32)?;
        let key = Secret32::from_bytes(key.as_slice().try_into().map_err(|_| Error::Archive)?)?;
        let encrypted = backup::read_private(&directory.join("account.backup"), 512)?;
        let plaintext = backup::open(domain(), &encrypted, &key)?;
        let mut reader = Reader::new(&plaintext);
        if reader.fixed::<8>()? != *b"ZUTTIACC" || reader.u16()? != 1 {
            return Err(Error::Archive);
        }
        let source = Option::<SpendingKey>::from(SpendingKey::from_bytes(reader.fixed()?))
            .ok_or(Error::Archive)?;
        let notes = NoteKeys::from_secrets(
            Secret32::from_bytes(reader.fixed()?)?,
            Secret32::from_bytes(reader.fixed()?)?,
            Secret32::from_bytes(reader.fixed()?)?,
        )?;
        reader.finish()?;
        Ok(Self { source, notes })
    }

    pub fn zcash_address(&self) -> Result<String> {
        let receiver = self.source_receiver();
        let address =
            Address::try_from_items(Revision::R0, vec![Uitem::Data(Receiver::Orchard(receiver))])
                .map_err(|_| Error::Archive)?;
        Ok(address.encode(&NetworkType::Test))
    }

    pub fn source_receiver(&self) -> [u8; 43] {
        self.source_viewing_key()
            .address_at(0_u32, Scope::External)
            .to_raw_address_bytes()
    }

    pub fn source_viewing_key(&self) -> FullViewingKey {
        FullViewingKey::from(&self.source)
    }

    pub fn source_spending_key(&self) -> &SpendingKey {
        &self.source
    }

    pub fn note_keys(&self) -> &NoteKeys {
        &self.notes
    }

    pub fn private_receiver(&self) -> Result<NoteRecipient> {
        Ok(self.notes.recipient()?)
    }

    pub fn sign_payment(
        &self,
        bytes: &[u8],
    ) -> Result<Vec<pczt::roles::signer::SpendAuthSignature>> {
        use orchard::keys::SpendAuthorizingKey;
        use pczt::{
            Pczt,
            roles::signer::{Signer, extract_orchard_spend_auth_signatures},
        };
        use rand::{rand_core::UnwrapErr, rngs::SysRng};
        let pczt = Pczt::parse(bytes).map_err(|_| Error::Pczt)?;
        let viewing = self.source_viewing_key();
        let mut owned = Vec::new();
        let pczt = pczt::roles::verifier::Verifier::new(pczt)
            .with_ironwood::<(), _>(|bundle| {
                bundle.verify_cross_address_restriction()?;
                for (index, action) in bundle.actions().iter().enumerate() {
                    if action
                        .spend()
                        .value()
                        .is_some_and(|value| value.inner() > 0)
                    {
                        action.spend().verify_nullifier(Some(&viewing))?;
                        action.spend().verify_rk(Some(&viewing))?;
                        owned.push(index);
                    }
                }
                Ok(())
            })
            .map_err(|_| Error::Signatures)?
            .finish();
        if owned.is_empty() {
            return Err(Error::Signatures);
        }
        let mut signer = Signer::new(pczt).map_err(|_| Error::Signatures)?;
        for index in &owned {
            signer
                .sign_ironwood(
                    UnwrapErr(SysRng),
                    *index,
                    &SpendAuthorizingKey::from(&self.source),
                )
                .map_err(|_| Error::Signatures)?;
        }
        Ok(extract_orchard_spend_auth_signatures(&signer.finish())
            .into_iter()
            .filter(|signature| {
                signature.value_pool() == orchard::ValuePool::Ironwood
                    && owned.contains(&signature.action_index())
            })
            .collect())
    }
}
