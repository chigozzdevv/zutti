use crate::{Result, args::ArchiveArgs};
use std::path::Path;
use zeroize::Zeroizing;
use zutti_protocol::{Digest, Secret32};
use zutti_wallet::backup;

pub fn key(path: &Path) -> Result<Secret32> {
    let bytes = backup::read_private(path, 32)?;
    Ok(Secret32::from_bytes(bytes.as_slice().try_into().map_err(
        |_| "key file must contain exactly 32 raw bytes",
    )?)?)
}

pub fn open(args: &ArchiveArgs, maximum: usize) -> Result<(Digest, Secret32, Zeroizing<Vec<u8>>)> {
    let domain = Digest::from_hex(&args.domain)?;
    let key = key(&args.key_file)?;
    let archive = backup::read_private(&args.backup, maximum + 128)?;
    let plaintext = backup::open(domain, &archive, &key)?;
    if plaintext.len() > maximum {
        return Err("recovery payload exceeds its bounds".into());
    }
    Ok((domain, key, plaintext))
}
