use crate::{
    Digest, Error, Result,
    encoding::{Canonical, Reader, Writer},
};
#[cfg(target_os = "zkvm")]
use risc0_zkvm::sha::rust_crypto::{Digest as _, Sha256};
#[cfg(not(target_os = "zkvm"))]
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OperationKind {
    Deposit = 1,
    Transfer = 2,
    Withdraw = 3,
    Reserve = 4,
    Settle = 5,
}
impl OperationKind {
    pub fn from_id(id: u8) -> Result<Self> {
        match id {
            1 => Ok(Self::Deposit),
            2 => Ok(Self::Transfer),
            3 => Ok(Self::Withdraw),
            4 => Ok(Self::Reserve),
            5 => Ok(Self::Settle),
            _ => Err(Error::Invalid("pool operation")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolClaim {
    pub kind: OperationKind,
    pub domain: Digest,
    pub root: Digest,
    pub checkpoint: Digest,
    pub order_tag: Digest,
    pub order: Digest,
    pub reservation: Digest,
    pub value: u64,
    pub owner: [u8; 20],
    pub nullifiers: Digest,
    pub outputs: Digest,
}
impl PoolClaim {
    pub fn validate(&self) -> Result<()> {
        Digest::nonzero(self.domain.0)?;
        Digest::nonzero(self.nullifiers.0)?;
        Digest::nonzero(self.outputs.0)?;
        let public_value = matches!(self.kind, OperationKind::Deposit | OperationKind::Withdraw);
        if public_value != (self.value != 0) || public_value != (self.owner != [0; 20]) {
            return Err(Error::Invalid("public pool value"));
        }
        if (self.kind == OperationKind::Deposit) != (self.root == Digest::default()) {
            return Err(Error::Invalid("pool membership root"));
        }
        if matches!(self.kind, OperationKind::Reserve | OperationKind::Settle)
            != (self.checkpoint != Digest::default())
        {
            return Err(Error::Invalid("source checkpoint"));
        }
        for field in [self.order_tag, self.order, self.reservation] {
            if (self.kind == OperationKind::Reserve) != (field != Digest::default()) {
                return Err(Error::Invalid("reservation binding"));
            }
        }
        if self.kind == OperationKind::Deposit && self.nullifiers != nullifier_digest(&[])? {
            return Err(Error::Invalid("deposit nullifiers"));
        }
        Ok(())
    }
    pub fn journal_digest(&self) -> Result<Digest> {
        Ok(Digest(Sha256::digest(self.to_bytes()?).into()))
    }
}
impl Canonical for PoolClaim {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        self.validate()?;
        writer.fixed(b"ZUTTIPOL");
        writer.u16(1);
        writer.u8(self.kind as u8);
        for digest in [
            self.domain,
            self.root,
            self.checkpoint,
            self.order_tag,
            self.order,
            self.reservation,
        ] {
            writer.fixed(&digest.0);
        }
        writer.u64(self.value);
        writer.fixed(&self.owner);
        writer.fixed(&self.nullifiers.0);
        writer.fixed(&self.outputs.0);
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTIPOL" || reader.u16()? != 1 {
            return Err(Error::Version);
        }
        let claim = Self {
            kind: OperationKind::from_id(reader.u8()?)?,
            domain: Digest(reader.fixed()?),
            root: Digest(reader.fixed()?),
            checkpoint: Digest(reader.fixed()?),
            order_tag: Digest(reader.fixed()?),
            order: Digest(reader.fixed()?),
            reservation: Digest(reader.fixed()?),
            value: reader.u64()?,
            owner: reader.fixed()?,
            nullifiers: Digest(reader.fixed()?),
            outputs: Digest(reader.fixed()?),
        };
        claim.validate()?;
        Ok(claim)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceUpdateClaim {
    pub genesis: Digest,
    pub policy: Digest,
    pub previous: Digest,
    pub previous_stable: Digest,
    pub state: Digest,
    pub checkpoint: Digest,
    pub work_be: [u8; 32],
}
impl Canonical for SourceUpdateClaim {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        for digest in [self.genesis, self.policy, self.state, self.checkpoint] {
            Digest::nonzero(digest.0)?;
        }
        if self.work_be == [0; 32] || self.state == self.previous {
            return Err(Error::Invalid("source transition"));
        }
        writer.fixed(b"ZUTTISRC");
        writer.u16(1);
        for digest in [
            self.genesis,
            self.policy,
            self.previous,
            self.previous_stable,
            self.state,
            self.checkpoint,
        ] {
            writer.fixed(&digest.0);
        }
        writer.fixed(&self.work_be);
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        if reader.fixed::<8>()? != *b"ZUTTISRC" || reader.u16()? != 1 {
            return Err(Error::Version);
        }
        let claim = Self {
            genesis: Digest(reader.fixed()?),
            policy: Digest(reader.fixed()?),
            previous: Digest(reader.fixed()?),
            previous_stable: Digest(reader.fixed()?),
            state: Digest(reader.fixed()?),
            checkpoint: Digest(reader.fixed()?),
            work_be: reader.fixed()?,
        };
        claim.to_bytes()?;
        Ok(claim)
    }
}

pub fn nullifier_digest(nullifiers: &[Digest]) -> Result<Digest> {
    if nullifiers.len() > 8 {
        return Err(Error::Length);
    }
    let mut hash = Sha256::new();
    let mut seen = std::collections::BTreeSet::new();
    for nullifier in nullifiers {
        Digest::nonzero(nullifier.0)?;
        if !seen.insert(*nullifier) {
            return Err(Error::Invalid("duplicate nullifier"));
        }
        hash.update(nullifier.0);
    }
    Ok(Digest(hash.finalize().into()))
}
pub fn output_digest(outputs: &[(Digest, Digest)]) -> Result<Digest> {
    if outputs.len() > 8 {
        return Err(Error::Length);
    }
    let mut hash = Sha256::new();
    let mut seen = std::collections::BTreeSet::new();
    for (commitment, ciphertext) in outputs {
        Digest::nonzero(commitment.0)?;
        Digest::nonzero(ciphertext.0)?;
        if !seen.insert(*commitment) {
            return Err(Error::Invalid("duplicate output"));
        }
        hash.update(commitment.0);
        hash.update(ciphertext.0);
    }
    Ok(Digest(hash.finalize().into()))
}

fn leaf(domain: Digest, commitment: Digest) -> Digest {
    let mut hash = Sha256::new();
    hash.update(b"ZUTTIPLF\x01\x00");
    hash.update(domain.0);
    hash.update(commitment.0);
    Digest(hash.finalize().into())
}
fn node(domain: Digest, level: u8, left: Digest, right: Digest) -> Digest {
    let mut hash = Sha256::new();
    hash.update(b"ZUTTIPND\x01\x00");
    hash.update(domain.0);
    hash.update([level]);
    hash.update(left.0);
    hash.update(right.0);
    Digest(hash.finalize().into())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolPath {
    pub position: u32,
    pub siblings: [Digest; 32],
}
impl PoolPath {
    pub fn verify(&self, domain: Digest, commitment: Digest, expected: Digest) -> Result<()> {
        Digest::nonzero(domain.0)?;
        Digest::nonzero(commitment.0)?;
        let mut result = leaf(domain, commitment);
        let mut index = self.position;
        for (level, sibling) in self.siblings.iter().enumerate() {
            result = if index & 1 == 0 {
                node(domain, level as u8, result, *sibling)
            } else {
                node(domain, level as u8, *sibling, result)
            };
            index >>= 1;
        }
        if result != expected {
            return Err(Error::Invalid("pool membership"));
        }
        Ok(())
    }
}
impl Canonical for PoolPath {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.u32(self.position);
        for sibling in self.siblings {
            writer.fixed(&sibling.0);
        }
        Ok(())
    }
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        let position = reader.u32()?;
        let mut siblings = [Digest::default(); 32];
        for sibling in &mut siblings {
            *sibling = Digest(reader.fixed()?);
        }
        Ok(Self { position, siblings })
    }
}

#[derive(Debug, Clone)]
pub struct PoolTree {
    domain: Digest,
    size: u64,
    empty: [Digest; 33],
    nodes: BTreeMap<(u8, u32), Digest>,
    commitments: std::collections::BTreeSet<Digest>,
}
impl PoolTree {
    pub fn new(domain: Digest) -> Result<Self> {
        Digest::nonzero(domain.0)?;
        let mut empty = [Digest::default(); 33];
        empty[0] = leaf(domain, Digest::default());
        for level in 0..32 {
            empty[level + 1] = node(domain, level as u8, empty[level], empty[level]);
        }
        Ok(Self {
            domain,
            size: 0,
            empty,
            nodes: BTreeMap::new(),
            commitments: std::collections::BTreeSet::new(),
        })
    }
    pub const fn size(&self) -> u64 {
        self.size
    }
    pub fn root(&self) -> Digest {
        self.at(32, 0)
    }
    pub fn append(&mut self, commitment: Digest) -> Result<u32> {
        Digest::nonzero(commitment.0)?;
        let position = u32::try_from(self.size).map_err(|_| Error::Length)?;
        if !self.commitments.insert(commitment) {
            return Err(Error::Invalid("duplicate pool commitment"));
        }
        let mut index = position;
        let mut value = leaf(self.domain, commitment);
        self.nodes.insert((0, index), value);
        for level in 0..32 {
            let sibling = self.at(level, index ^ 1);
            value = if index & 1 == 0 {
                node(self.domain, level, value, sibling)
            } else {
                node(self.domain, level, sibling, value)
            };
            index >>= 1;
            self.nodes.insert((level + 1, index), value);
        }
        self.size += 1;
        Ok(position)
    }
    pub fn prove(&self, position: u32) -> Result<PoolPath> {
        if u64::from(position) >= self.size {
            return Err(Error::Invalid("pool position"));
        }
        let mut index = position;
        let mut siblings = [Digest::default(); 32];
        for (level, sibling) in siblings.iter_mut().enumerate() {
            *sibling = self.at(level as u8, index ^ 1);
            index >>= 1;
        }
        Ok(PoolPath { position, siblings })
    }
    fn at(&self, level: u8, index: u32) -> Digest {
        self.nodes
            .get(&(level, index))
            .copied()
            .unwrap_or(self.empty[usize::from(level)])
    }
}
