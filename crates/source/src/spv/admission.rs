use super::{Error, HEADER_BYTES, HeaderClaim, MAX_HEADERS, Result};
use zutti_protocol::{
    Digest,
    encoding::{Canonical, Reader, Writer},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AdmissionKind {
    Advance = 1,
    Conflict = 2,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainPoint {
    pub prefix: Digest,
    pub height: u32,
    pub hash: Digest,
    pub timestamp: u32,
    pub work: [u8; 32],
}
impl ChainPoint {
    pub fn from_header(claim: &HeaderClaim) -> Result<Self> {
        Ok(Self {
            prefix: claim.id()?,
            height: claim.height()?,
            hash: claim.tip()?,
            timestamp: claim.timestamp()?,
            work: claim.work()?,
        })
    }
}
impl Canonical for ChainPoint {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        Digest::nonzero(self.prefix.0)?;
        Digest::nonzero(self.hash.0)?;
        if self.work == [0; 32] || self.height >= 500_000_000 {
            return Err(zutti_protocol::Error::Invalid("SPV chain point"));
        }
        writer.fixed(&self.prefix.0);
        writer.u32(self.height);
        writer.fixed(&self.hash.0);
        writer.u32(self.timestamp);
        writer.fixed(&self.work);
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        let point = Self {
            prefix: Digest(reader.fixed()?),
            height: reader.u32()?,
            hash: Digest(reader.fixed()?),
            timestamp: reader.u32()?,
            work: reader.fixed()?,
        };
        point.to_bytes()?;
        Ok(point)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionClaim {
    pub kind: AdmissionKind,
    pub policy: Digest,
    pub previous: Digest,
    pub stable: ChainPoint,
    pub head: ChainPoint,
    pub maximum_time: u32,
}
impl Canonical for AdmissionClaim {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        Digest::nonzero(self.policy.0)?;
        if self.stable.height >= self.head.height
            || self.stable.work >= self.head.work
            || self.stable.prefix == self.head.prefix
            || self.maximum_time < self.stable.timestamp
            || self.maximum_time < self.head.timestamp
            || (self.kind == AdmissionKind::Conflict
                && (self.previous == Digest::default() || self.previous == self.stable.prefix))
        {
            return Err(zutti_protocol::Error::Invalid("SPV admission claim"));
        }
        writer.fixed(b"ZUTTISPA");
        writer.u16(1);
        writer.u8(self.kind as u8);
        writer.fixed(&self.policy.0);
        writer.fixed(&self.previous.0);
        self.stable.encode(writer)?;
        self.head.encode(writer)?;
        writer.u32(self.maximum_time);
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTISPA" || reader.u16()? != 1 {
            return Err(zutti_protocol::Error::Version);
        }
        let kind = match reader.u8()? {
            1 => AdmissionKind::Advance,
            2 => AdmissionKind::Conflict,
            _ => return Err(zutti_protocol::Error::Version),
        };
        let claim = Self {
            kind,
            policy: Digest(reader.fixed()?),
            previous: Digest(reader.fixed()?),
            stable: ChainPoint::decode(reader)?,
            head: ChainPoint::decode(reader)?,
            maximum_time: reader.u32()?,
        };
        claim.to_bytes()?;
        Ok(claim)
    }
}

#[derive(Debug, Clone)]
pub enum AdmissionWitness {
    Initialize {
        stable: HeaderClaim,
        confirmations: Vec<Vec<u8>>,
    },
    Advance {
        previous: HeaderClaim,
        stable_headers: Vec<Vec<u8>>,
        confirmations: Vec<Vec<u8>>,
    },
    Conflict {
        alternative: HeaderClaim,
        accepted_stable: Digest,
        headers: Vec<Vec<u8>>,
    },
}
impl AdmissionWitness {
    pub fn predecessor(&self) -> &HeaderClaim {
        match self {
            Self::Initialize { stable, .. } => stable,
            Self::Advance { previous, .. } => previous,
            Self::Conflict { alternative, .. } => alternative,
        }
    }
    fn check_headers(groups: &[&[Vec<u8>]]) -> Result<()> {
        let count: usize = groups.iter().map(|g| g.len()).sum();
        if count == 0
            || count > MAX_HEADERS
            || groups
                .iter()
                .flat_map(|g| g.iter())
                .any(|h| h.len() != HEADER_BYTES)
        {
            return Err(Error::Bounds);
        }
        Ok(())
    }
    pub fn evaluate(&self) -> Result<AdmissionClaim> {
        let policy = self.predecessor().policy();
        let (kind, previous, stable, head) = match self {
            Self::Initialize {
                stable,
                confirmations,
            } => {
                Self::check_headers(&[confirmations])?;
                if stable.height()? == 0 || confirmations.len() != policy.confirmations() as usize {
                    return Err(Error::Confirmations);
                }
                (
                    AdmissionKind::Advance,
                    Digest::default(),
                    stable.clone(),
                    stable.advance(confirmations)?,
                )
            }
            Self::Advance {
                previous,
                stable_headers,
                confirmations,
            } => {
                Self::check_headers(&[stable_headers, confirmations])?;
                if stable_headers.is_empty()
                    || confirmations.len() != policy.confirmations() as usize
                {
                    return Err(Error::Confirmations);
                }
                let stable = previous.advance(stable_headers)?;
                let head = stable.advance(confirmations)?;
                (
                    AdmissionKind::Advance,
                    if previous.height()? == 0 {
                        Digest::default()
                    } else {
                        previous.id()?
                    },
                    stable,
                    head,
                )
            }
            Self::Conflict {
                alternative,
                accepted_stable,
                headers,
            } => {
                Self::check_headers(&[headers])?;
                if headers.len() < policy.confirmations() as usize
                    || *accepted_stable == Digest::default()
                    || alternative.id()? == *accepted_stable
                {
                    return Err(Error::Policy);
                }
                (
                    AdmissionKind::Conflict,
                    *accepted_stable,
                    alternative.clone(),
                    alternative.advance(headers)?,
                )
            }
        };
        let claim = AdmissionClaim {
            kind,
            policy: policy.id()?,
            previous,
            stable: ChainPoint::from_header(&stable)?,
            head: ChainPoint::from_header(&head)?,
            maximum_time: head.maximum_time(),
        };
        claim.to_bytes()?;
        Ok(claim)
    }
}

fn write_headers(writer: &mut Writer, headers: &[Vec<u8>]) {
    writer.u32(headers.len() as u32);
    for header in headers {
        writer.fixed(header);
    }
}
fn read_headers(reader: &mut Reader<'_>) -> zutti_protocol::Result<Vec<Vec<u8>>> {
    let count = reader.u32()? as usize;
    if count > MAX_HEADERS {
        return Err(zutti_protocol::Error::Length);
    }
    let mut headers = Vec::with_capacity(count);
    for _ in 0..count {
        headers.push(reader.fixed::<HEADER_BYTES>()?.to_vec());
    }
    Ok(headers)
}
impl Canonical for AdmissionWitness {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        writer.fixed(b"ZUTTISAI");
        writer.u16(1);
        match self {
            Self::Initialize {
                stable,
                confirmations,
            } => {
                Self::check_headers(&[confirmations]).map_err(|_| zutti_protocol::Error::Length)?;
                writer.u8(3);
                stable.encode(writer)?;
                write_headers(writer, confirmations);
            }
            Self::Advance {
                previous,
                stable_headers,
                confirmations,
            } => {
                Self::check_headers(&[stable_headers, confirmations])
                    .map_err(|_| zutti_protocol::Error::Length)?;
                writer.u8(1);
                previous.encode(writer)?;
                write_headers(writer, stable_headers);
                write_headers(writer, confirmations);
            }
            Self::Conflict {
                alternative,
                accepted_stable,
                headers,
            } => {
                Self::check_headers(&[headers]).map_err(|_| zutti_protocol::Error::Length)?;
                writer.u8(2);
                alternative.encode(writer)?;
                writer.fixed(&Digest::nonzero(accepted_stable.0)?.0);
                write_headers(writer, headers);
            }
        }
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTISAI" || reader.u16()? != 1 {
            return Err(zutti_protocol::Error::Version);
        }
        let witness = match reader.u8()? {
            3 => Self::Initialize {
                stable: HeaderClaim::decode(reader)?,
                confirmations: read_headers(reader)?,
            },
            1 => Self::Advance {
                previous: HeaderClaim::decode(reader)?,
                stable_headers: read_headers(reader)?,
                confirmations: read_headers(reader)?,
            },
            2 => Self::Conflict {
                alternative: HeaderClaim::decode(reader)?,
                accepted_stable: Digest::nonzero(reader.fixed()?)?,
                headers: read_headers(reader)?,
            },
            _ => return Err(zutti_protocol::Error::Version),
        };
        witness.to_bytes()?;
        Ok(witness)
    }
    fn from_bytes(bytes: &[u8]) -> zutti_protocol::Result<Self> {
        if bytes.is_empty() || bytes.len() > super::MAX_WITNESS {
            return Err(zutti_protocol::Error::Length);
        }
        let mut reader = Reader::new(bytes);
        let witness = Self::decode(&mut reader)?;
        reader.finish()?;
        Ok(witness)
    }
}
