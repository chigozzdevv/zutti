use super::{
    CoverageClaim, CoverageWitness, HEADER_BYTES, HeaderClaim, MAX_HEADERS, Policy, Result,
    WindowClaim, WindowWitness,
};
use zutti_protocol::encoding::{Canonical, Reader, Writer};

pub const MAX_WITNESS: usize = 16 * 1024 * 1024;

#[derive(Debug)]
pub enum Witness {
    Coverage(Box<CoverageWitness>),
    Genesis {
        policy: Policy,
        header: Vec<u8>,
        headers: Vec<Vec<u8>>,
    },
    Headers {
        previous: HeaderClaim,
        headers: Vec<Vec<u8>>,
    },
    Window {
        previous: HeaderClaim,
        window: WindowWitness,
    },
}

#[derive(Debug)]
pub enum Evaluation {
    Coverage(CoverageClaim),
    Headers(HeaderClaim),
    Window(WindowClaim),
}

impl Evaluation {
    pub fn journal(&self) -> zutti_protocol::Result<Vec<u8>> {
        match self {
            Self::Headers(claim) => claim.to_bytes(),
            Self::Coverage(claim) => claim.to_bytes(),
            Self::Window(claim) => claim.to_bytes(),
        }
    }
}

impl Witness {
    pub fn policy(&self) -> Policy {
        match self {
            Self::Genesis { policy, .. } => *policy,
            Self::Coverage(witness) => witness.previous.head().policy(),
            Self::Headers { previous, .. } | Self::Window { previous, .. } => previous.policy(),
        }
    }
    pub fn required_journal(&self) -> zutti_protocol::Result<Option<Vec<u8>>> {
        match self {
            Self::Genesis { .. } => Ok(None),
            Self::Headers { previous, .. } | Self::Window { previous, .. } => {
                Ok(Some(previous.to_bytes()?))
            }
            Self::Coverage(witness) => Ok(Some(witness.previous.journal()?)),
        }
    }
    pub fn is_private(&self) -> bool {
        matches!(self, Self::Window { .. } | Self::Coverage(_))
    }
    pub fn evaluate(&self) -> Result<Evaluation> {
        match self {
            Self::Coverage(witness) => witness.evaluate(),
            Self::Genesis {
                policy,
                header,
                headers,
            } => {
                let start = HeaderClaim::genesis(*policy, header)?;
                Ok(Evaluation::Headers(if headers.is_empty() {
                    start
                } else {
                    start.advance(headers)?
                }))
            }
            Self::Headers { previous, headers } => {
                Ok(Evaluation::Headers(previous.advance(headers)?))
            }
            Self::Window { previous, window } => Ok(Evaluation::Window(window.evaluate(previous)?)),
        }
    }
}

impl Canonical for Witness {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        writer.fixed(b"ZUTTISPI");
        writer.u16(1);
        match self {
            Self::Coverage(witness) => {
                writer.u8(4);
                witness.encode(writer)?;
            }
            Self::Genesis {
                policy,
                header,
                headers,
            } => {
                if header.len() != HEADER_BYTES
                    || headers.len() > MAX_HEADERS
                    || headers.iter().any(|h| h.len() != HEADER_BYTES)
                {
                    return Err(zutti_protocol::Error::Length);
                }
                writer.u8(1);
                policy.encode(writer)?;
                writer.fixed(header);
                writer.u32(headers.len() as u32);
                for header in headers {
                    writer.fixed(header);
                }
            }
            Self::Headers { previous, headers } => {
                if headers.is_empty()
                    || headers.len() > MAX_HEADERS
                    || headers.iter().any(|h| h.len() != HEADER_BYTES)
                {
                    return Err(zutti_protocol::Error::Length);
                }
                writer.u8(2);
                previous.encode(writer)?;
                writer.u32(headers.len() as u32);
                for header in headers {
                    writer.fixed(header);
                }
            }
            Self::Window { previous, window } => {
                writer.u8(3);
                previous.encode(writer)?;
                window.encode(writer)?;
            }
        }
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTISPI" || reader.u16()? != 1 {
            return Err(zutti_protocol::Error::Version);
        }
        match reader.u8()? {
            4 => Ok(Self::Coverage(Box::new(CoverageWitness::decode(reader)?))),
            1 => {
                let policy = Policy::decode(reader)?;
                let header = reader.fixed::<HEADER_BYTES>()?.to_vec();
                let count = reader.u32()? as usize;
                if count > MAX_HEADERS {
                    return Err(zutti_protocol::Error::Length);
                }
                let mut headers = Vec::with_capacity(count);
                for _ in 0..count {
                    headers.push(reader.fixed::<HEADER_BYTES>()?.to_vec());
                }
                Ok(Self::Genesis {
                    policy,
                    header,
                    headers,
                })
            }
            2 => {
                let previous = HeaderClaim::decode(reader)?;
                let count = reader.u32()? as usize;
                if count == 0 || count > MAX_HEADERS {
                    return Err(zutti_protocol::Error::Length);
                }
                let mut headers = Vec::with_capacity(count);
                for _ in 0..count {
                    headers.push(reader.fixed::<HEADER_BYTES>()?.to_vec());
                }
                Ok(Self::Headers { previous, headers })
            }
            3 => Ok(Self::Window {
                previous: HeaderClaim::decode(reader)?,
                window: WindowWitness::decode(reader)?,
            }),
            _ => Err(zutti_protocol::Error::Version),
        }
    }
    fn from_bytes(bytes: &[u8]) -> zutti_protocol::Result<Self> {
        if bytes.is_empty() || bytes.len() > MAX_WITNESS {
            return Err(zutti_protocol::Error::Length);
        }
        let mut reader = Reader::new(bytes);
        let witness = Self::decode(&mut reader)?;
        reader.finish()?;
        Ok(witness)
    }
}
