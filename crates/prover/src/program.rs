use crate::{Error, artifact::persist_bounded, backend::admitted_program};
use std::{io::Read, path::Path, sync::Arc};
use zutti_protocol::{Digest, statement::ProofKind};

pub const MAX_PROGRAM: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ProgramKind {
    Source,
    SourceUpdate,
    Pool,
    Preparation,
    HistoryQuery,
}
impl ProgramKind {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::SourceUpdate => "source-update",
            Self::Pool => "pool",
            Self::Preparation => "preparation",
            Self::HistoryQuery => "history-query",
        }
    }
    fn proof_kind(self) -> ProofKind {
        match self {
            Self::Source => ProofKind::SourceTransition,
            Self::SourceUpdate => ProofKind::Source,
            Self::Pool => ProofKind::Note,
            Self::Preparation => ProofKind::Preparation,
            Self::HistoryQuery => ProofKind::HistoryQuery,
        }
    }
}

#[derive(Clone)]
enum Code {
    Embedded(&'static [u8]),
    Loaded(Arc<[u8]>),
}
#[derive(Clone)]
pub struct Program {
    kind: ProgramKind,
    image: Digest,
    bytes: Code,
}
impl Program {
    pub fn current(kind: ProgramKind) -> Self {
        let (id, bytes): (_, &'static [u8]) = match kind {
            ProgramKind::Source => (zutti_methods::SOURCE_ID, zutti_methods::SOURCE_ELF),
            ProgramKind::SourceUpdate => (
                zutti_methods::SOURCE_UPDATE_ID,
                zutti_methods::SOURCE_UPDATE_ELF,
            ),
            ProgramKind::Pool => (zutti_methods::POOL_ID, zutti_methods::POOL_ELF),
            ProgramKind::Preparation => (
                zutti_methods::PREPARATION_ID,
                zutti_methods::PREPARATION_ELF,
            ),
            ProgramKind::HistoryQuery => (
                zutti_methods::HISTORY_QUERY_ID,
                zutti_methods::HISTORY_QUERY_ELF,
            ),
        };
        Self {
            kind,
            image: Digest(risc0_zkvm::sha::Digest::from(id).into()),
            bytes: Code::Embedded(bytes),
        }
    }
    pub fn read(kind: ProgramKind, path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let file = std::fs::File::open(path)?;
        if !file.metadata()?.is_file() {
            return Err("program must be a regular file".into());
        }
        let mut bytes = Vec::new();
        file.take(MAX_PROGRAM as u64 + 1).read_to_end(&mut bytes)?;
        Ok(Self::from_bytes(kind, bytes)?)
    }
    pub fn from_bytes(kind: ProgramKind, bytes: Vec<u8>) -> Result<Self, Error> {
        if bytes.is_empty() || bytes.len() > MAX_PROGRAM {
            return Err(Error::Binding);
        }
        let embedded = Self::current(kind);
        if bytes == embedded.bytes() {
            return Ok(embedded);
        }
        let image = Digest(
            risc0_zkvm::compute_image_id(&bytes)
                .map_err(|_| Error::Binding)?
                .into(),
        );
        if !admitted_program(kind.proof_kind(), image) {
            return Err(Error::BackendUnavailable);
        }
        Ok(Self {
            kind,
            image,
            bytes: Code::Loaded(bytes.into()),
        })
    }
    pub fn export_new(&self, directory: &Path) -> Result<(), Box<dyn std::error::Error>> {
        std::fs::create_dir(directory)?;
        persist_bounded(&directory.join("program.elf"), self.bytes(), MAX_PROGRAM)?;
        let manifest = serde_json::json!({"version":1,"kind":self.kind.name(),"image_id":self.image.to_string(),"program":"program.elf"});
        persist_bounded(
            &directory.join("manifest.json"),
            &serde_json::to_vec_pretty(&manifest)?,
            4_096,
        )
    }
    pub const fn kind(&self) -> ProgramKind {
        self.kind
    }
    pub const fn image(&self) -> Digest {
        self.image
    }
    pub fn bytes(&self) -> &[u8] {
        match &self.bytes {
            Code::Embedded(bytes) => bytes,
            Code::Loaded(bytes) => bytes,
        }
    }
}

impl std::fmt::Debug for Program {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Program")
            .field("kind", &self.kind)
            .field("image", &self.image)
            .field("bytes", &self.bytes().len())
            .finish()
    }
}
