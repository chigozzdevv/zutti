use super::{Error, MAX_HEADERS, Policy, Result, parse_header};
use crate::difficulty::HeaderCandidate;
use zutti_protocol::{
    Digest,
    commitment::hash,
    encoding::{Canonical, Reader, Writer},
};

#[derive(Debug, Clone)]
pub struct HeaderClaim {
    policy: Policy,
    headers: HeaderCandidate,
}

impl HeaderClaim {
    pub fn genesis(policy: Policy, bytes: &[u8]) -> Result<Self> {
        policy.to_bytes()?;
        let header = parse_header(bytes)?;
        let headers = HeaderCandidate::from_genesis_header(&header, policy.network())?;
        Ok(Self { policy, headers })
    }
    pub fn advance(&self, headers: &[Vec<u8>]) -> Result<Self> {
        if headers.is_empty() || headers.len() > MAX_HEADERS {
            return Err(Error::Bounds);
        }
        self.to_bytes()?;
        if u64::from(self.height()?) + headers.len() as u64 > u64::from(self.policy.valid_through())
        {
            return Err(Error::Policy);
        }
        let mut next = self.clone();
        for bytes in headers {
            next.headers.append_header(&parse_header(bytes)?)?;
        }
        Ok(next)
    }
    pub const fn policy(&self) -> Policy {
        self.policy
    }
    pub fn height(&self) -> Result<u32> {
        Ok(self.headers.height()?)
    }
    pub fn tip(&self) -> Result<Digest> {
        Ok(self.headers.tip()?)
    }
    pub fn work(&self) -> Result<[u8; 32]> {
        Ok(self.headers.cumulative_work()?)
    }
    pub fn id(&self) -> zutti_protocol::Result<Digest> {
        Ok(hash(b"spv-header-claim", &[&self.to_bytes()?]))
    }
}

impl Canonical for HeaderClaim {
    fn encode(&self, writer: &mut Writer) -> zutti_protocol::Result<()> {
        if self.headers.network() != self.policy.network()
            || self
                .headers
                .height()
                .map_err(|_| zutti_protocol::Error::Invalid("SPV header height"))?
                > self.policy.valid_through()
        {
            return Err(zutti_protocol::Error::Invalid("SPV header network"));
        }
        writer.fixed(b"ZUTTISPH");
        writer.u16(1);
        self.policy.encode(writer)?;
        self.headers.encode(writer)
    }
    fn decode(reader: &mut Reader<'_>) -> zutti_protocol::Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTISPH" || reader.u16()? != 1 {
            return Err(zutti_protocol::Error::Version);
        }
        let claim = Self {
            policy: Policy::decode(reader)?,
            headers: HeaderCandidate::decode(reader)?,
        };
        claim.to_bytes()?;
        Ok(claim)
    }
}
