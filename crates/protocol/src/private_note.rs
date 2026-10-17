use crate::{
    Digest, Error, Result, Secret32,
    amount::{Amount, Asset},
    commitment::hash,
    encoding::{Canonical, Reader, Writer},
};
use k256::ecdsa::{Signature, VerifyingKey, signature::hazmat::PrehashVerifier};
use zeroize::Zeroizing;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct NoteRecipient {
    owner: [u8; 33],
    viewing: [u8; 32],
    nullifier_key: Digest,
}

impl core::fmt::Debug for NoteRecipient {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("NoteRecipient([REDACTED])")
    }
}

impl NoteRecipient {
    pub fn new(owner: [u8; 33], viewing: [u8; 32], nullifier_key: Digest) -> Result<Self> {
        VerifyingKey::from_sec1_bytes(&owner).map_err(|_| Error::Invalid("note owner"))?;
        Digest::nonzero(viewing)?;
        let mut modulus = [255; 32];
        modulus[0] = 237;
        modulus[31] = 127;
        if viewing.iter().rev().cmp(modulus.iter().rev()) != core::cmp::Ordering::Less {
            return Err(Error::Invalid("noncanonical note viewing key"));
        }
        Digest::nonzero(nullifier_key.0)?;
        Ok(Self {
            owner,
            viewing,
            nullifier_key,
        })
    }
    pub const fn owner(&self) -> &[u8; 33] {
        &self.owner
    }
    pub const fn viewing(&self) -> &[u8; 32] {
        &self.viewing
    }
    pub const fn nullifier_key(&self) -> Digest {
        self.nullifier_key
    }
}

impl Canonical for NoteRecipient {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.fixed(&self.owner);
        writer.fixed(&self.viewing);
        writer.fixed(&self.nullifier_key.0);
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Self::new(reader.fixed()?, reader.fixed()?, Digest(reader.fixed()?))
    }
}

pub struct PrivateNote {
    domain: Digest,
    recipient: NoteRecipient,
    amount: Amount,
    rho: Secret32,
    blinding: Secret32,
}

impl core::fmt::Debug for PrivateNote {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("PrivateNote([REDACTED])")
    }
}

impl PrivateNote {
    pub fn new(
        domain: Digest,
        recipient: NoteRecipient,
        amount: Amount,
        rho: Secret32,
        blinding: Secret32,
    ) -> Result<Self> {
        Digest::nonzero(domain.0)?;
        amount.positive()?;
        if amount.asset() != Asset::Usdc {
            return Err(Error::Asset);
        }
        Ok(Self {
            domain,
            recipient,
            amount,
            rho,
            blinding,
        })
    }
    pub const fn domain(&self) -> Digest {
        self.domain
    }
    pub const fn recipient(&self) -> NoteRecipient {
        self.recipient
    }
    pub const fn amount(&self) -> Amount {
        self.amount
    }
    pub fn commitment(&self) -> Result<Digest> {
        let bytes = Zeroizing::new(self.to_bytes()?);
        Ok(hash(b"private-note", &[&bytes]))
    }
    pub fn nullifier(&self, key: &Secret32) -> Result<Digest> {
        if nullifier_key_commitment(key) != self.recipient.nullifier_key {
            return Err(Error::Invalid("note nullifier key"));
        }
        Ok(hash(
            b"private-note-nullifier",
            &[
                &self.domain.0,
                &self.commitment()?.0,
                self.rho.expose(),
                key.expose(),
            ],
        ))
    }
    pub fn spend_digest(&self, operation: Digest) -> Result<Digest> {
        Digest::nonzero(operation.0)?;
        Ok(hash(
            b"private-note-spend",
            &[&self.domain.0, &self.commitment()?.0, &operation.0],
        ))
    }
    pub fn verify_spend(
        &self,
        operation: Digest,
        authorization: &SpendAuthorization,
    ) -> Result<()> {
        let signature = Signature::from_slice(&authorization.0)
            .map_err(|_| Error::Invalid("note authorization"))?;
        if signature.normalize_s().is_some() {
            return Err(Error::Invalid("noncanonical note authorization"));
        }
        VerifyingKey::from_sec1_bytes(self.recipient.owner())
            .map_err(|_| Error::Invalid("note owner"))?
            .verify_prehash(&self.spend_digest(operation)?.0, &signature)
            .map_err(|_| Error::Invalid("note authorization"))
    }
}

pub fn nullifier_key_commitment(key: &Secret32) -> Digest {
    hash(b"private-note-nullifier-key", &[key.expose()])
}

impl Canonical for PrivateNote {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.fixed(b"ZUTTINOT");
        writer.u16(1);
        writer.fixed(&self.domain.0);
        self.recipient.encode(writer)?;
        self.amount.encode(writer)?;
        writer.fixed(self.rho.expose());
        writer.fixed(self.blinding.expose());
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTINOT" || reader.u16()? != 1 {
            return Err(Error::Version);
        }
        Self::new(
            Digest(reader.fixed()?),
            NoteRecipient::decode(reader)?,
            Amount::decode(reader)?,
            Secret32::from_bytes(reader.fixed()?)?,
            Secret32::from_bytes(reader.fixed()?)?,
        )
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct SpendAuthorization([u8; 64]);

impl core::fmt::Debug for SpendAuthorization {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SpendAuthorization([REDACTED])")
    }
}

impl SpendAuthorization {
    pub fn from_bytes(bytes: [u8; 64]) -> Result<Self> {
        let signature =
            Signature::from_slice(&bytes).map_err(|_| Error::Invalid("note authorization"))?;
        if signature.normalize_s().is_some() {
            return Err(Error::Invalid("noncanonical note authorization"));
        }
        Ok(Self(bytes))
    }
    pub const fn as_bytes(&self) -> &[u8; 64] {
        &self.0
    }
}

impl Canonical for SpendAuthorization {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.fixed(&self.0);
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Self::from_bytes(reader.fixed()?)
    }
}
