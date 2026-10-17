use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use zeroize::Zeroizing;
use zutti_protocol::{
    Digest, Error, Result, Secret32,
    commitment::hash,
    encoding::{Canonical, Reader, Writer},
};

pub struct EscrowNote {
    pub domain: Digest,
    pub order: Digest,
    pub reservation: Digest,
    pub amount: u128,
    secret: Secret32,
    blinding: Secret32,
}
impl core::fmt::Debug for EscrowNote {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("EscrowNote([REDACTED])")
    }
}
impl EscrowNote {
    pub fn new(
        domain: Digest,
        order: Digest,
        reservation: Digest,
        amount: u128,
        secret: Secret32,
        blinding: Secret32,
    ) -> Result<Self> {
        for digest in [domain, order, reservation] {
            Digest::nonzero(digest.0)?;
        }
        if amount == 0 {
            return Err(Error::Invalid("escrow amount"));
        }
        Ok(Self {
            domain,
            order,
            reservation,
            amount,
            secret,
            blinding,
        })
    }
    pub fn commitment(&self) -> Result<Digest> {
        Ok(hash(
            b"conditional-escrow",
            &[&Zeroizing::new(self.to_bytes()?)],
        ))
    }
    pub fn nullifier(&self) -> Result<Digest> {
        Ok(hash(
            b"conditional-escrow-nullifier",
            &[&self.domain.0, &self.commitment()?.0, self.secret.expose()],
        ))
    }
    fn envelope_key(domain: Digest, commitment: Digest, secret: &Secret32) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(
            hash(
                b"conditional-escrow-envelope",
                &[&domain.0, &commitment.0, secret.expose()],
            )
            .0,
        )
    }
    fn aad(domain: Digest, commitment: Digest, nonce: &[u8; 24]) -> Vec<u8> {
        let mut writer = Writer::default();
        writer.fixed(b"ZUTTIECT");
        writer.u16(1);
        writer.fixed(&domain.0);
        writer.fixed(&commitment.0);
        writer.fixed(nonce);
        writer.finish()
    }
    pub fn encrypt(&self, nonce: [u8; 24]) -> Result<Vec<u8>> {
        let commitment = self.commitment()?;
        let key = Self::envelope_key(self.domain, commitment, &self.secret);
        let aad = Self::aad(self.domain, commitment, &nonce);
        let plaintext = Zeroizing::new(self.to_bytes()?);
        let ciphertext = XChaCha20Poly1305::new_from_slice(key.as_ref())
            .map_err(|_| Error::Invalid("escrow key"))?
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &plaintext,
                    aad: &aad,
                },
            )
            .map_err(|_| Error::Invalid("escrow encryption"))?;
        let mut writer = Writer::default();
        writer.fixed(&aad);
        writer.bytes(&ciphertext)?;
        Ok(writer.finish())
    }
    pub fn decrypt(
        bytes: &[u8],
        domain: Digest,
        commitment: Digest,
        secret: &Secret32,
    ) -> Result<Self> {
        let mut reader = Reader::new(bytes);
        if reader.fixed::<8>()? != *b"ZUTTIECT"
            || reader.u16()? != 1
            || reader.fixed::<32>()? != domain.0
            || reader.fixed::<32>()? != commitment.0
        {
            return Err(Error::Invalid("escrow envelope context"));
        }
        let nonce = reader.fixed()?;
        let ciphertext = reader.bytes(256)?;
        reader.finish()?;
        let key = Self::envelope_key(domain, commitment, secret);
        let plaintext = Zeroizing::new(
            XChaCha20Poly1305::new_from_slice(key.as_ref())
                .map_err(|_| Error::Invalid("escrow key"))?
                .decrypt(
                    XNonce::from_slice(&nonce),
                    Payload {
                        msg: ciphertext,
                        aad: &Self::aad(domain, commitment, &nonce),
                    },
                )
                .map_err(|_| Error::Invalid("escrow authentication"))?,
        );
        let note = Self::from_bytes(&plaintext)?;
        if note.domain != domain
            || note.commitment()? != commitment
            || note.secret.expose() != secret.expose()
        {
            return Err(Error::Invalid("escrow plaintext"));
        }
        Ok(note)
    }
}
impl Canonical for EscrowNote {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.fixed(b"ZUTTIESC");
        writer.u16(1);
        writer.fixed(&self.domain.0);
        writer.fixed(&self.order.0);
        writer.fixed(&self.reservation.0);
        writer.u128(self.amount);
        writer.fixed(self.secret.expose());
        writer.fixed(self.blinding.expose());
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTIESC" || reader.u16()? != 1 {
            return Err(Error::Version);
        }
        Self::new(
            Digest(reader.fixed()?),
            Digest(reader.fixed()?),
            Digest(reader.fixed()?),
            reader.u128()?,
            Secret32::from_bytes(reader.fixed()?)?,
            Secret32::from_bytes(reader.fixed()?)?,
        )
    }
}
