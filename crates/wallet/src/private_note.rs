use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use hkdf::Hkdf;
use k256::ecdsa::{Signature, SigningKey, signature::hazmat::PrehashSigner};
use sha2::Digest as _;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroizing;
use zutti_protocol::{
    Digest, Error, Result, Secret32,
    commitment::hash,
    encoding::{Canonical, Reader, Writer},
    note::FixedOutput,
    private_note::{NoteRecipient, PrivateNote, SpendAuthorization, nullifier_key_commitment},
};

const NOTE_BYTES: usize = 8 + 2 + 32 + 33 + 32 + 32 + 1 + 16 + 32 + 32;
const CIPHERTEXT_BYTES: usize = NOTE_BYTES + 16;

pub struct NoteKeys {
    spending: SigningKey,
    viewing: StaticSecret,
    nullifier: Secret32,
}

impl core::fmt::Debug for NoteKeys {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("NoteKeys([REDACTED])")
    }
}

impl NoteKeys {
    pub fn from_secrets(
        spending: Secret32,
        viewing: Secret32,
        nullifier: Secret32,
    ) -> Result<Self> {
        Ok(Self {
            spending: SigningKey::from_slice(spending.expose())
                .map_err(|_| Error::Invalid("note spending key"))?,
            viewing: StaticSecret::from(*viewing.expose()),
            nullifier,
        })
    }
    pub fn recipient(&self) -> Result<NoteRecipient> {
        let owner = self.spending.verifying_key().to_encoded_point(true);
        NoteRecipient::new(
            owner.as_bytes().try_into().map_err(|_| Error::Length)?,
            PublicKey::from(&self.viewing).to_bytes(),
            nullifier_key_commitment(&self.nullifier),
        )
    }
    pub fn authorize(&self, note: &PrivateNote, operation: Digest) -> Result<SpendAuthorization> {
        if note.recipient() != self.recipient()? {
            return Err(Error::Invalid("note owner mismatch"));
        }
        let signature: Signature = self
            .spending
            .sign_prehash(&note.spend_digest(operation)?.0)
            .map_err(|_| Error::Invalid("note authorization"))?;
        SpendAuthorization::from_bytes(signature.to_bytes().into())
    }
    pub fn nullifier(&self, note: &PrivateNote) -> Result<Digest> {
        if note.recipient() != self.recipient()? {
            return Err(Error::Invalid("note owner mismatch"));
        }
        note.nullifier(&self.nullifier)
    }
    pub fn decrypt(&self, encrypted: &EncryptedNote, domain: Digest) -> Result<PrivateNote> {
        decrypt(encrypted, &self.viewing, domain)
    }
}

pub struct NoteEncryption {
    ephemeral: StaticSecret,
    nonce: [u8; 24],
}

impl core::fmt::Debug for NoteEncryption {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("NoteEncryption([REDACTED])")
    }
}

impl NoteEncryption {
    pub fn from_entropy(ephemeral: Secret32, nonce: [u8; 24]) -> Self {
        Self {
            ephemeral: StaticSecret::from(*ephemeral.expose()),
            nonce,
        }
    }
    pub fn random() -> Result<Self> {
        let mut nonce = [0; 24];
        getrandom::fill(&mut nonce).map_err(|_| Error::Entropy)?;
        Ok(Self::from_entropy(Secret32::random()?, nonce))
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct EncryptedNote {
    domain: Digest,
    commitment: Digest,
    ephemeral: [u8; 32],
    nonce: [u8; 24],
    ciphertext: Vec<u8>,
}

impl core::fmt::Debug for EncryptedNote {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("EncryptedNote")
            .field("ciphertext_bytes", &self.ciphertext.len())
            .finish_non_exhaustive()
    }
}

impl EncryptedNote {
    pub fn encrypt(note: &PrivateNote) -> Result<Self> {
        Self::encrypt_with(note, NoteEncryption::random()?)
    }
    pub fn encrypt_with(note: &PrivateNote, randomness: NoteEncryption) -> Result<Self> {
        let recipient = PublicKey::from(*note.recipient().viewing());
        let mut envelope = Self {
            domain: note.domain(),
            commitment: note.commitment()?,
            ephemeral: PublicKey::from(&randomness.ephemeral).to_bytes(),
            nonce: randomness.nonce,
            ciphertext: Vec::new(),
        };
        let key = key(&randomness.ephemeral, &recipient, &envelope)?;
        let plaintext = Zeroizing::new(note.to_bytes()?);
        let aad = envelope.aad(recipient.as_bytes());
        envelope.ciphertext = XChaCha20Poly1305::new_from_slice(key.as_ref())
            .map_err(|_| Error::Invalid("note encryption key"))?
            .encrypt(
                XNonce::from_slice(&envelope.nonce),
                Payload {
                    msg: &plaintext,
                    aad: &aad,
                },
            )
            .map_err(|_| Error::Invalid("note encryption"))?;
        Ok(envelope)
    }
    pub const fn domain(&self) -> Digest {
        self.domain
    }
    pub const fn commitment(&self) -> Digest {
        self.commitment
    }
    pub fn digest(&self) -> Result<Digest> {
        Ok(Digest(sha2::Sha256::digest(self.to_bytes()?).into()))
    }
    pub fn fixed_output(&self, note: &PrivateNote) -> Result<FixedOutput> {
        if note.domain() != self.domain || note.commitment()? != self.commitment {
            return Err(Error::Invalid("note ciphertext binding"));
        }
        FixedOutput::new(self.commitment, self.digest()?, note.amount())
    }
    fn aad(&self, recipient: &[u8; 32]) -> Vec<u8> {
        let mut writer = Writer::default();
        writer.fixed(b"ZUTTIENC");
        writer.u16(1);
        writer.fixed(&self.domain.0);
        writer.fixed(&self.commitment.0);
        writer.fixed(&self.ephemeral);
        writer.fixed(&self.nonce);
        writer.fixed(recipient);
        writer.finish()
    }
}

impl Canonical for EncryptedNote {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.fixed(b"ZUTTIENC");
        writer.u16(1);
        writer.fixed(&self.domain.0);
        writer.fixed(&self.commitment.0);
        writer.fixed(&self.ephemeral);
        writer.fixed(&self.nonce);
        writer.bytes(&self.ciphertext)
    }
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTIENC" || reader.u16()? != 1 {
            return Err(Error::Version);
        }
        let domain = Digest::nonzero(reader.fixed()?)?;
        let commitment = Digest::nonzero(reader.fixed()?)?;
        let ephemeral = Digest::nonzero(reader.fixed()?)?.0;
        let nonce = reader.fixed()?;
        let ciphertext = reader.bytes(CIPHERTEXT_BYTES)?.to_vec();
        if ciphertext.len() != CIPHERTEXT_BYTES {
            return Err(Error::Length);
        }
        Ok(Self {
            domain,
            commitment,
            ephemeral,
            nonce,
            ciphertext,
        })
    }
}

pub fn decrypt_with_viewing_key(
    encrypted: &EncryptedNote,
    viewing: &Secret32,
    domain: Digest,
) -> Result<PrivateNote> {
    decrypt(encrypted, &StaticSecret::from(*viewing.expose()), domain)
}

fn decrypt(
    encrypted: &EncryptedNote,
    viewing: &StaticSecret,
    domain: Digest,
) -> Result<PrivateNote> {
    if encrypted.domain != domain {
        return Err(Error::Invalid("note domain mismatch"));
    }
    let ephemeral = PublicKey::from(encrypted.ephemeral);
    let recipient = PublicKey::from(viewing);
    let key = key(viewing, &ephemeral, encrypted)?;
    let aad = encrypted.aad(recipient.as_bytes());
    let plaintext = Zeroizing::new(
        XChaCha20Poly1305::new_from_slice(key.as_ref())
            .map_err(|_| Error::Invalid("note encryption key"))?
            .decrypt(
                XNonce::from_slice(&encrypted.nonce),
                Payload {
                    msg: &encrypted.ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| Error::Invalid("note decryption"))?,
    );
    let note = PrivateNote::from_bytes(&plaintext)?;
    if note.domain() != domain
        || note.commitment()? != encrypted.commitment
        || note.recipient().viewing() != recipient.as_bytes()
    {
        return Err(Error::Invalid("note plaintext binding"));
    }
    Ok(note)
}

fn key(
    secret: &StaticSecret,
    public: &PublicKey,
    envelope: &EncryptedNote,
) -> Result<Zeroizing<[u8; 32]>> {
    let shared = secret.diffie_hellman(public);
    if !shared.was_contributory() {
        return Err(Error::Invalid("noncontributory note key"));
    }
    let mut key = Zeroizing::new([0; 32]);
    let info = hash(
        b"private-note-encryption-key",
        &[&envelope.commitment.0, &envelope.ephemeral],
    );
    Hkdf::<sha2::Sha256>::new(Some(&envelope.domain.0), shared.as_bytes())
        .expand(&info.0, key.as_mut())
        .map_err(|_| Error::Invalid("note key derivation"))?;
    Ok(key)
}
