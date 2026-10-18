use super::{Error, Result};
use crate::transition::TransitionWitness;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::BTreeSet;
use zutti_protocol::Digest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeMetadata {
    pub revision: i64,
    pub policy: Vec<u8>,
    pub state: Vec<u8>,
    pub genesis: Vec<u8>,
    pub base: Vec<u8>,
    pub evidence: Option<Vec<u8>>,
    pub next_id: i64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeRecord {
    pub id: i64,
    pub claim: Vec<u8>,
    pub witness: Option<Vec<u8>>,
    pub evidence: Option<Vec<u8>>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeArchive {
    pub metadata: RangeMetadata,
    pub batches: Vec<RangeRecord>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeafRecord {
    pub tree: u8,
    pub key: Digest,
    pub value: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeafChange {
    pub tree: u8,
    pub key: Digest,
    pub value: Option<Vec<u8>>,
}
#[derive(Debug)]
pub struct RangeCommit {
    pub expected_revision: i64,
    pub metadata: RangeMetadata,
    pub batches: Vec<RangeRecord>,
    pub removed: Vec<i64>,
    pub through: Option<i64>,
    pub leaves: Vec<LeafChange>,
}
pub trait RangeDurability: std::fmt::Debug + Send {
    fn commit(&self, update: &RangeCommit) -> Result<()>;
}
pub(super) fn metadata(connection: &Connection) -> Result<RangeMetadata> {
    Ok(connection.query_row(
        "SELECT revision,policy,state,genesis,base,evidence,next_id FROM range_metadata WHERE id=1",
        [],
        |row| {
            Ok(RangeMetadata {
                revision: row.get(0)?,
                policy: row.get(1)?,
                state: row.get(2)?,
                genesis: row.get(3)?,
                base: row.get(4)?,
                evidence: row.get(5)?,
                next_id: row.get(6)?,
            })
        },
    )?)
}
pub(super) fn record(connection: &Connection, id: i64) -> Result<RangeRecord> {
    Ok(connection.query_row(
        "SELECT id,claim,witness,evidence FROM range_batches WHERE id=?1",
        [id],
        |row| {
            Ok(RangeRecord {
                id: row.get(0)?,
                claim: row.get(1)?,
                witness: row.get(2)?,
                evidence: row.get(3)?,
            })
        },
    )?)
}
pub(super) fn archive(connection: &Connection) -> Result<RangeArchive> {
    let metadata = metadata(connection)?;
    let mut statement = connection.prepare("SELECT id FROM range_batches ORDER BY id")?;
    let ids = statement
        .query_map([], |row| row.get::<_, i64>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(RangeArchive {
        metadata,
        batches: ids
            .into_iter()
            .map(|id| record(connection, id))
            .collect::<Result<Vec<_>>>()?,
    })
}
pub(super) fn write_archive(connection: &Connection, archive: &RangeArchive) -> Result<()> {
    let m = &archive.metadata;
    connection.execute(
        "INSERT INTO range_metadata VALUES(1,?1,?2,?3,?4,?5,?6,?7)",
        params![
            m.revision, m.policy, m.state, m.genesis, m.base, m.evidence, m.next_id
        ],
    )?;
    for row in &archive.batches {
        connection.execute(
            "INSERT INTO range_batches VALUES(?1,?2,?3,?4)",
            params![row.id, row.claim, row.witness, row.evidence],
        )?;
    }
    Ok(())
}
pub(super) fn changed_leaves(
    connection: &Connection,
    witness: &TransitionWitness,
) -> Result<Vec<LeafChange>> {
    let mut keys = BTreeSet::new();
    for (tree, transcript) in [
        (0, &witness.ledger),
        (1, &witness.spends),
        (3, &witness.archive),
    ] {
        keys.extend(transcript.accesses().iter().map(|a| (tree, a.key())));
    }
    keys.extend(witness.index.iter().map(|p| (2, p.key())));
    keys.into_iter()
        .map(|(tree, key)| {
            let value = connection
                .query_row(
                    "SELECT value FROM leaves WHERE tree=?1 AND key=?2",
                    params![tree, key.0.as_slice()],
                    |row| row.get::<_, Vec<u8>>(0),
                )
                .optional()?;
            Ok(LeafChange { tree, key, value })
        })
        .collect()
}
pub(super) fn commit(
    transaction: rusqlite::Transaction<'_>,
    durability: Option<&dyn RangeDurability>,
    expected_revision: i64,
    batches: &[i64],
    removed: &[i64],
    through: Option<i64>,
    leaves: Vec<LeafChange>,
) -> Result<()> {
    if let Some(durability) = durability {
        let metadata = metadata(&transaction)?;
        if metadata.revision != expected_revision.checked_add(1).ok_or(Error::Binding)? {
            return Err(Error::Binding);
        }
        durability.commit(&RangeCommit {
            expected_revision,
            metadata,
            batches: batches
                .iter()
                .map(|id| record(&transaction, *id))
                .collect::<Result<Vec<_>>>()?,
            removed: removed.to_vec(),
            through,
            leaves,
        })?;
    }
    transaction.commit()?;
    Ok(())
}
