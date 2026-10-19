use super::{APPLICATION, Error, Result, connect, initialize, persist_delta, restore_candidate};
use crate::{
    bootstrap::GenesisBlock,
    proof::{SourcePolicy, SourceReceiptClaim},
    range::{SourceRangeClaim, SourceRangeWitness},
    state::{SourceCandidate, TransitionError},
    transition::{MAX_WITNESS, StateSnapshot},
};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use std::path::Path;
use zutti_protocol::encoding::Canonical;

#[derive(Debug)]
pub struct ReplayStore {
    connection: rusqlite::Connection,
    policy: SourcePolicy,
    candidate: SourceCandidate,
    revision: i64,
}
impl ReplayStore {
    pub fn create(path: &Path, genesis: &[u8], policy: SourcePolicy) -> Result<Self> {
        let genesis =
            GenesisBlock::parse(genesis, policy.network).map_err(TransitionError::from)?;
        let candidate = SourceCandidate::from_genesis(&genesis, policy.id()?, policy.index_start)?;
        let state = candidate.state_snapshot()?.to_bytes()?;
        let connection = initialize(path, &candidate, 3, |tx| {
            tx.execute_batch(
                "CREATE TABLE replay_metadata (id INTEGER PRIMARY KEY CHECK(id=1),
                    revision INTEGER NOT NULL, policy BLOB NOT NULL, state BLOB NOT NULL,
                    height INTEGER NOT NULL CHECK(height>=0));
                 CREATE TABLE replay_samples (start_height INTEGER PRIMARY KEY CHECK(start_height>=0),
                    end_height INTEGER NOT NULL CHECK(end_height>start_height), witness BLOB NOT NULL);",
            )?;
            tx.execute(
                "INSERT INTO replay_metadata VALUES(1,0,?1,?2,0)",
                params![policy.to_bytes()?, state],
            )?;
            Ok(())
        })?;
        Ok(Self {
            connection,
            policy,
            candidate,
            revision: 0,
        })
    }
    pub fn open(path: &Path, policy: SourcePolicy) -> Result<Self> {
        let mut connection = connect(path)?;
        if connection.pragma_query_value(None, "application_id", |r| r.get::<_, i32>(0))?
            != APPLICATION
            || connection.pragma_query_value(None, "user_version", |r| r.get::<_, i32>(0))? != 3
        {
            return Err(Error::Binding);
        }
        let tx = connection.transaction()?;
        let (revision, stored_policy, state, height) = tx.query_row(
            "SELECT revision,policy,state,height FROM replay_metadata WHERE id=1",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, u32>(3)?,
                ))
            },
        )?;
        if revision < 0 || stored_policy != policy.to_bytes()? || state.len() > MAX_WITNESS {
            return Err(Error::Binding);
        }
        let state = StateSnapshot::from_bytes(&state)?;
        if state.height().map_err(TransitionError::from)? != height {
            return Err(Error::Binding);
        }
        let candidate = restore_candidate(&tx, policy, &state)?;
        tx.commit()?;
        Ok(Self {
            connection,
            policy,
            candidate,
            revision,
        })
    }
    pub const fn candidate(&self) -> &SourceCandidate {
        &self.candidate
    }
    pub const fn policy(&self) -> SourcePolicy {
        self.policy
    }
    pub fn advance(&mut self, blocks: Vec<Vec<u8>>) -> Result<SourceRangeClaim> {
        self.commit(blocks, false)
    }
    pub fn sample(&mut self, blocks: Vec<Vec<u8>>) -> Result<SourceRangeClaim> {
        self.commit(blocks, true)
    }
    fn commit(&mut self, blocks: Vec<Vec<u8>>, retain: bool) -> Result<SourceRangeClaim> {
        let mut next = self.candidate.clone();
        let transition = next.record_transition(blocks)?;
        let state = next.state_snapshot()?;
        let claim = SourceRangeClaim::new(
            SourceReceiptClaim::from_state(self.policy, transition.before())?,
            SourceReceiptClaim::from_state(self.policy, &state)?,
        )?;
        let witness = if retain {
            Some(SourceRangeWitness::transition(self.policy, transition.clone())?.to_bytes()?)
        } else {
            None
        };
        let revision = self.revision.checked_add(1).ok_or(Error::Binding)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if tx.execute(
            "UPDATE replay_metadata SET revision=?1,state=?2,height=?3 WHERE id=1 AND revision=?4",
            params![revision, state.to_bytes()?, claim.end.height, self.revision],
        )? != 1
        {
            return Err(Error::Stale);
        }
        persist_delta(&tx, &next, &transition)?;
        if let Some(witness) = witness {
            tx.execute(
                "INSERT INTO replay_samples VALUES(?1,?2,?3)",
                params![claim.start.height, claim.end.height, witness],
            )?;
        }
        tx.commit()?;
        self.candidate = next;
        self.revision = revision;
        Ok(claim)
    }
    pub fn samples(&self) -> Result<Vec<(u32, u32)>> {
        let mut query = self
            .connection
            .prepare("SELECT start_height,end_height FROM replay_samples ORDER BY start_height")?;
        Ok(query
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub fn witness(&self, start: u32) -> Result<Option<SourceRangeWitness>> {
        let bytes = self
            .connection
            .query_row(
                "SELECT witness FROM replay_samples WHERE start_height=?1",
                [start],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?;
        bytes
            .map(|bytes| {
                if bytes.len() > MAX_WITNESS {
                    return Err(Error::Binding);
                }
                let witness = SourceRangeWitness::from_bytes(&bytes)?;
                if witness.policy() != self.policy {
                    return Err(Error::Binding);
                }
                let claim = witness.evaluate()?;
                let end = self.connection.query_row(
                    "SELECT end_height FROM replay_samples WHERE start_height=?1",
                    [start],
                    |row| row.get::<_, u32>(0),
                )?;
                if claim.start.height != start
                    || claim.end.height != end
                    || end > self.candidate.height().map_err(TransitionError::from)?
                {
                    return Err(Error::Binding);
                }
                Ok(witness)
            })
            .transpose()
    }
}
