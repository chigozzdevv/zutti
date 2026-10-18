use crate::{
    Digest, Error, Result,
    commitment::hash,
    encoding::{Canonical, Reader, Writer},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ProofKind {
    Source = 1,
    Preparation = 2,
    Note = 3,
    Reservation = 4,
    Settlement = 5,
    HistoryQuery = 6,
    SourceTransition = 7,
    SourceRange = 8,
}
impl ProofKind {
    pub fn from_id(id: u8) -> Result<Self> {
        match id {
            1 => Ok(Self::Source),
            2 => Ok(Self::Preparation),
            3 => Ok(Self::Note),
            4 => Ok(Self::Reservation),
            5 => Ok(Self::Settlement),
            6 => Ok(Self::HistoryQuery),
            7 => Ok(Self::SourceTransition),
            8 => Ok(Self::SourceRange),
            _ => Err(Error::Invalid("proof kind")),
        }
    }
}
#[derive(Clone, PartialEq, Eq)]
pub struct Statement {
    kind: ProofKind,
    domain: Digest,
    program: Digest,
    payload: Vec<u8>,
}
impl core::fmt::Debug for Statement {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Statement")
            .field("kind", &self.kind)
            .field("payload_bytes", &self.payload.len())
            .finish_non_exhaustive()
    }
}
impl Statement {
    pub fn new(kind: ProofKind, domain: Digest, program: Digest, payload: Vec<u8>) -> Result<Self> {
        Digest::nonzero(domain.0)?;
        Digest::nonzero(program.0)?;
        if payload.is_empty() || payload.len() > 65_536 {
            return Err(Error::Length);
        }
        Ok(Self {
            kind,
            domain,
            program,
            payload,
        })
    }
    pub const fn kind(&self) -> ProofKind {
        self.kind
    }
    pub const fn domain(&self) -> Digest {
        self.domain
    }
    pub const fn program(&self) -> Digest {
        self.program
    }
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
    pub fn digest(&self) -> Result<Digest> {
        Ok(hash(b"proof-statement", &[&self.to_bytes()?]))
    }
}
impl Canonical for Statement {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.fixed(b"ZUTTISTM");
        writer.u16(1);
        writer.u8(self.kind as u8);
        writer.fixed(&self.domain.0);
        writer.fixed(&self.program.0);
        writer.bytes(&self.payload)
    }
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTISTM" || reader.u16()? != 1 {
            return Err(Error::Version);
        }
        Self::new(
            ProofKind::from_id(reader.u8()?)?,
            Digest(reader.fixed()?),
            Digest(reader.fixed()?),
            reader.bytes(65_536)?.to_vec(),
        )
    }
}
