use crate::{Error, ProofEnvelope, ProofVerifier, VerifiedProof, backend::MAX_PROOF};
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
};
use zutti_protocol::{
    encoding::{Canonical, Reader, Writer},
    statement::Statement,
};

pub const MAX_ARTIFACT: usize = MAX_PROOF + 66_560;

#[derive(Debug)]
pub struct ProofArtifact {
    statement: Statement,
    proof: ProofEnvelope,
}
impl ProofArtifact {
    pub fn new(statement: Statement, proof: ProofEnvelope) -> Result<Self, Error> {
        proof.check_binding(&statement)?;
        Ok(Self { statement, proof })
    }
    pub const fn statement(&self) -> &Statement {
        &self.statement
    }
    pub const fn proof(&self) -> &ProofEnvelope {
        &self.proof
    }
    pub fn verify(&self, expected: &Statement) -> Result<VerifiedProof, Error> {
        ProofVerifier.verify(&self.proof, expected)
    }
    pub fn read(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let file = File::open(path)?;
        if !file.metadata()?.is_file() {
            return Err("proof input must be a regular file".into());
        }
        let mut bytes = Vec::new();
        file.take(MAX_ARTIFACT as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > MAX_ARTIFACT {
            return Err("proof artifact exceeds its size limit".into());
        }
        Ok(Self::from_bytes(&bytes)?)
    }
    pub fn persist_new(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let bytes = self.to_bytes()?;
        if bytes.len() > MAX_ARTIFACT {
            return Err("proof artifact exceeds its size limit".into());
        }
        persist_bounded(path, &bytes, MAX_ARTIFACT)
    }
}
impl Canonical for ProofArtifact {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        writer.fixed(b"ZUTTIART");
        writer.u16(1);
        writer.bytes(&self.statement.to_bytes()?)?;
        writer.bytes(&self.proof.to_bytes()?)
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTIART" || reader.u16()? != 1 {
            return Err(zutti_protocol::Error::Version);
        }
        let statement = Statement::from_bytes(reader.bytes(65_664)?)?;
        let proof = ProofEnvelope::from_bytes(reader.bytes(MAX_PROOF + 128)?)?;
        Self::new(statement, proof)
            .map_err(|_| zutti_protocol::Error::Invalid("artifact statement binding"))
    }
}

pub(crate) fn persist_bounded(
    path: &Path,
    bytes: &[u8],
    limit: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    if bytes.is_empty() || bytes.len() > limit {
        return Err("artifact exceeds its size bound".into());
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist_noclobber(path)
        .map_err(|error| error.error)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}
