use crate::{Error, Result};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};
use zeroize::{Zeroize, Zeroizing};
use zutti_protocol::{
    Digest, Secret32,
    encoding::{Reader, Writer},
};

pub const MAX_PLAINTEXT: usize = 16 * 1024 * 1024;
pub const MAX_ARCHIVE: usize = MAX_PLAINTEXT + 128;

pub fn seal(domain: Digest, plaintext: &[u8], key: &Secret32) -> Result<Vec<u8>> {
    Digest::nonzero(domain.0)?;
    if plaintext.is_empty() || plaintext.len() > MAX_PLAINTEXT {
        return Err(Error::Archive);
    }
    let mut nonce = [0; 24];
    getrandom::fill(&mut nonce).map_err(|_| zutti_protocol::Error::Entropy)?;
    let mut header = Writer::default();
    header.fixed(b"ZUTTIBKP");
    header.u16(1);
    header.fixed(&domain.0);
    header.fixed(&nonce);
    let header = header.finish();
    let cipher = XChaCha20Poly1305::new(key.expose().into());
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: &header,
            },
        )
        .map_err(|_| Error::Archive)?;
    let mut output = Writer::default();
    output.fixed(&header);
    output.bytes(&ciphertext)?;
    Ok(output.finish())
}

pub fn open(domain: Digest, archive: &[u8], key: &Secret32) -> Result<Zeroizing<Vec<u8>>> {
    if archive.len() > MAX_ARCHIVE {
        return Err(Error::Archive);
    }
    let mut reader = Reader::new(archive);
    if reader.fixed::<8>()? != *b"ZUTTIBKP"
        || reader.u16()? != 1
        || Digest(reader.fixed()?) != domain
    {
        return Err(Error::Archive);
    }
    let nonce = reader.fixed::<24>()?;
    let ciphertext = reader.bytes(MAX_PLAINTEXT + 16)?;
    reader.finish()?;
    let cipher = XChaCha20Poly1305::new(key.expose().into());
    let mut plaintext = cipher
        .decrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: ciphertext,
                aad: &archive[..66],
            },
        )
        .map_err(|_| Error::Archive)?;
    if plaintext.is_empty() || plaintext.len() > MAX_PLAINTEXT {
        plaintext.zeroize();
        return Err(Error::Archive);
    }
    Ok(Zeroizing::new(plaintext))
}

pub fn persist_new(path: &Path, archive: &[u8]) -> Result<()> {
    if archive.is_empty() || archive.len() > MAX_ARCHIVE {
        return Err(Error::Archive);
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    temporary.write_all(archive)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist_noclobber(path)
        .map_err(|e| Error::Io(e.error))?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

pub fn read_private(path: &Path, max: usize) -> Result<Zeroizing<Vec<u8>>> {
    let limit = u64::try_from(max)
        .ok()
        .and_then(|n| n.checked_add(1))
        .ok_or(Error::Archive)?;
    if std::fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(Error::Permissions);
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > max as u64 {
        return Err(Error::Permissions);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(Error::Permissions);
        }
    }
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(limit).read_to_end(&mut bytes)?;
    if bytes.len() > max {
        return Err(Error::Archive);
    }
    Ok(bytes)
}
