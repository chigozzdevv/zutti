use super::durable::{self, LeafRecord, RangeArchive, RangeDurability};
use super::{
    APPLICATION, Error, MAX_EVIDENCE, Result, connect, initialize, persist_delta, restore_candidate,
};
use crate::{
    bootstrap::GenesisBlock,
    proof::{SourceInput, SourcePolicy, SourceReceiptClaim, SourceWitness},
    range::{SourceRangeClaim, SourceRangeWitness},
    state::SourceCandidate,
    transition::{MAX_WITNESS, StateSnapshot},
};
use rusqlite::{Connection, TransactionBehavior, params};
use std::path::Path;
use zutti_protocol::encoding::Canonical;

const RANGE_SCHEMA: &str = "CREATE TABLE range_metadata (id INTEGER PRIMARY KEY CHECK(id=1), revision INTEGER NOT NULL,
                    policy BLOB NOT NULL, state BLOB NOT NULL, genesis BLOB NOT NULL, base BLOB NOT NULL,
                    evidence BLOB, next_id INTEGER NOT NULL CHECK(next_id>0));
                CREATE TABLE range_batches (id INTEGER PRIMARY KEY CHECK(id>0), claim BLOB NOT NULL,
                    witness BLOB, evidence BLOB, CHECK((witness IS NULL)!=(evidence IS NULL)));";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RangeLimits {
    pub batches: usize,
    pub witness_bytes: usize,
}
impl RangeLimits {
    fn validate(self) -> Result<()> {
        if !(1..=64).contains(&self.batches)
            || self.witness_bytes == 0
            || self.witness_bytes > 512 * 1024 * 1024
        {
            return Err(Error::Binding);
        }
        Ok(())
    }
}
impl Default for RangeLimits {
    fn default() -> Self {
        Self {
            batches: 16,
            witness_bytes: 128 * 1024 * 1024,
        }
    }
}
#[derive(Debug, Clone)]
pub struct RangeBatch {
    pub id: u64,
    pub claim: SourceRangeClaim,
    pub witness: Option<SourceRangeWitness>,
    pub evidence: Option<Vec<u8>>,
}
#[derive(Debug)]
pub struct RangeStore {
    connection: Connection,
    policy: SourcePolicy,
    candidate: SourceCandidate,
    genesis: Vec<u8>,
    base: SourceReceiptClaim,
    evidence: Option<Vec<u8>>,
    batches: Vec<RangeBatch>,
    limits: RangeLimits,
    revision: i64,
    next_id: i64,
    durability: Option<Box<dyn RangeDurability>>,
}
impl RangeStore {
    pub fn create(
        path: &Path,
        genesis: Vec<u8>,
        policy: SourcePolicy,
        limits: RangeLimits,
    ) -> Result<Self> {
        limits.validate()?;
        let parsed =
            GenesisBlock::parse(&genesis, policy.network).map_err(super::TransitionError::from)?;
        let candidate = SourceCandidate::from_genesis(&parsed, policy.id()?, policy.index_start)?;
        let snapshot = candidate.state_snapshot()?;
        let base = SourceReceiptClaim::from_state(policy, &snapshot)?;
        let connection = initialize(path, &candidate, 2, |tx| {
            tx.execute_batch(RANGE_SCHEMA)?;
            tx.execute(
                "INSERT INTO range_metadata VALUES(1,0,?1,?2,?3,?4,NULL,1)",
                params![
                    policy.to_bytes()?,
                    snapshot.to_bytes()?,
                    genesis,
                    base.to_bytes()?
                ],
            )?;
            Ok(())
        })?;
        Ok(Self {
            connection,
            policy,
            candidate,
            genesis,
            base,
            evidence: None,
            batches: Vec::new(),
            limits,
            revision: 0,
            next_id: 1,
            durability: None,
        })
    }
    pub fn open(path: &Path, policy: SourcePolicy, limits: RangeLimits) -> Result<Self> {
        Self::open_connection(connect(path)?, policy, limits)
    }
    fn open_connection(
        mut connection: Connection,
        policy: SourcePolicy,
        limits: RangeLimits,
    ) -> Result<Self> {
        limits.validate()?;
        if connection.pragma_query_value(None, "application_id", |r| r.get::<_, i32>(0))?
            != APPLICATION
            || connection.pragma_query_value(None, "user_version", |r| r.get::<_, i32>(0))? != 2
        {
            return Err(Error::Binding);
        }
        let tx = connection.transaction()?;
        let (revision, encoded_policy, state, genesis, base, evidence, next_id) = tx.query_row(
            "SELECT revision,policy,state,genesis,base,evidence,next_id FROM range_metadata WHERE id=1", [], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?, row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, Vec<u8>>(3)?, row.get::<_, Vec<u8>>(4)?, row.get::<_, Option<Vec<u8>>>(5)?, row.get::<_, i64>(6)?))
            })?;
        if revision < 0
            || next_id < 1
            || encoded_policy != policy.to_bytes()?
            || state.len() > MAX_WITNESS
            || genesis.len() > MAX_WITNESS
        {
            return Err(Error::Binding);
        }
        if let Some(bytes) = &evidence {
            check_evidence(bytes)?;
        }
        let base = SourceReceiptClaim::from_bytes(&base)?;
        check_claim(policy, &base)?;
        let initial = SourceWitness {
            policy,
            input: SourceInput::Genesis(genesis.clone()),
        }
        .evaluate()?;
        if (base.height == 0 || evidence.is_none()) && base != initial {
            return Err(Error::Binding);
        }
        let state = StateSnapshot::from_bytes(&state)?;
        let candidate = restore_candidate(&tx, policy, &state)?;
        let mut batches = Vec::new();
        let mut witness_bytes = 0usize;
        let mut previous = base.clone();
        {
            let mut statement =
                tx.prepare("SELECT id,claim,witness,evidence FROM range_batches ORDER BY id")?;
            let mut rows = statement.query([])?;
            while let Some(row) = rows.next()? {
                let id = row.get::<_, i64>(0)?;
                let claim = SourceRangeClaim::from_bytes(&row.get::<_, Vec<u8>>(1)?)?;
                let witness = row.get::<_, Option<Vec<u8>>>(2)?;
                let evidence = row.get::<_, Option<Vec<u8>>>(3)?;
                if id <= 0
                    || id >= next_id
                    || claim.start != previous
                    || witness.is_some() == evidence.is_some()
                {
                    return Err(Error::Binding);
                }
                claim.require_policy(policy)?;
                let witness = if let Some(bytes) = witness {
                    witness_bytes = witness_bytes
                        .checked_add(bytes.len())
                        .ok_or(Error::Binding)?;
                    if bytes.len() > MAX_WITNESS || witness_bytes > limits.witness_bytes {
                        return Err(Error::QueueFull);
                    }
                    let witness = SourceRangeWitness::from_bytes(&bytes)?;
                    if !matches!(witness, SourceRangeWitness::Transition(_))
                        || witness.policy() != policy
                        || witness.evaluate()? != claim
                    {
                        return Err(Error::Binding);
                    }
                    Some(witness)
                } else {
                    None
                };
                if let Some(bytes) = &evidence {
                    check_evidence(bytes)?;
                }
                previous = claim.end.clone();
                batches.push(RangeBatch {
                    id: id as u64,
                    claim,
                    witness,
                    evidence,
                });
                if batches.len() > limits.batches {
                    return Err(Error::QueueFull);
                }
            }
        }
        if previous != SourceReceiptClaim::from_state(policy, &state)? {
            return Err(Error::Binding);
        }
        tx.commit()?;
        Ok(Self {
            connection,
            policy,
            candidate,
            genesis,
            base,
            evidence,
            batches,
            limits,
            revision,
            next_id,
            durability: None,
        })
    }
    pub fn archive(&self) -> Result<RangeArchive> {
        durable::archive(&self.connection)
    }
    pub fn attach_durability(&mut self, durability: Box<dyn RangeDurability>) {
        self.durability = Some(durability);
    }
    pub fn for_each_leaf(&self, visit: &mut dyn FnMut(LeafRecord) -> Result<()>) -> Result<()> {
        for tree in 0..4 {
            super::for_each_leaf(&self.connection, tree, |key, value| {
                visit(LeafRecord {
                    tree,
                    key,
                    value: value.to_vec(),
                })
            })?;
        }
        Ok(())
    }
    pub fn restore(
        path: &Path,
        archive: &RangeArchive,
        policy: SourcePolicy,
        limits: RangeLimits,
        load: impl FnOnce(&mut dyn FnMut(LeafRecord) -> Result<()>) -> Result<()>,
    ) -> Result<Self> {
        let connection = super::initialize_rows(path, 2, |connection| {
            connection.execute_batch(RANGE_SCHEMA)?;
            durable::write_archive(connection, archive)?;
            load(&mut |record| {
                if record.tree > 3 {
                    return Err(Error::Binding);
                }
                super::put(connection, record.tree, record.key, Some(&record.value))
            })
        })?;
        Self::open_connection(connection, policy, limits)
    }
    pub const fn policy(&self) -> SourcePolicy {
        self.policy
    }
    pub const fn candidate(&self) -> &SourceCandidate {
        &self.candidate
    }
    pub fn genesis(&self) -> &[u8] {
        &self.genesis
    }
    pub const fn base(&self) -> &SourceReceiptClaim {
        &self.base
    }
    pub fn evidence(&self) -> Option<&[u8]> {
        self.evidence.as_deref()
    }
    pub fn batches(&self) -> &[RangeBatch] {
        &self.batches
    }
    pub fn batch(&self, id: u64) -> Result<&RangeBatch> {
        self.batches
            .iter()
            .find(|batch| batch.id == id)
            .ok_or(Error::Binding)
    }
    pub fn preview(&self, blocks: Vec<Vec<u8>>) -> Result<SourceRangeWitness> {
        self.prepare(blocks).map(|(_, witness)| witness)
    }
    fn prepare(&self, blocks: Vec<Vec<u8>>) -> Result<(SourceCandidate, SourceRangeWitness)> {
        if self.batches.len() >= self.limits.batches {
            return Err(Error::QueueFull);
        }
        let mut next = self.candidate.clone();
        let witness = SourceRangeWitness::transition(self.policy, next.record_transition(blocks)?)?;
        let size = witness.to_bytes()?.len();
        let pending = self
            .batches
            .iter()
            .filter_map(|b| b.witness.as_ref())
            .try_fold(0usize, |total, w| {
                total.checked_add(w.to_bytes()?.len()).ok_or(Error::Binding)
            })?;
        if size > MAX_WITNESS
            || pending
                .checked_add(size)
                .is_none_or(|n| n > self.limits.witness_bytes)
        {
            return Err(Error::QueueFull);
        }
        Ok((next, witness))
    }
    pub fn advance(&mut self, blocks: Vec<Vec<u8>>) -> Result<u64> {
        let (next, witness) = self.prepare(blocks)?;
        let claim = witness.evaluate()?;
        let next_id = self.next_id.checked_add(1).ok_or(Error::Binding)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if tx.execute("UPDATE range_metadata SET revision=revision+1,state=?1,next_id=?2 WHERE id=1 AND revision=?3",
            params![next.state_snapshot()?.to_bytes()?, next_id, self.revision])? != 1 { return Err(Error::Stale); }
        let SourceRangeWitness::Transition(source) = &witness else {
            return Err(Error::Binding);
        };
        let SourceInput::Advance { transition, .. } = &source.input else {
            return Err(Error::Binding);
        };
        persist_delta(&tx, &next, transition)?;
        tx.execute(
            "INSERT INTO range_batches VALUES(?1,?2,?3,NULL)",
            params![self.next_id, claim.to_bytes()?, witness.to_bytes()?],
        )?;
        let leaves = durable::changed_leaves(&tx, transition)?;
        durable::commit(
            tx,
            self.durability.as_deref(),
            self.revision,
            &[self.next_id],
            &[],
            None,
            leaves,
        )?;
        let id = self.next_id as u64;
        self.candidate = next;
        self.batches.push(RangeBatch {
            id,
            claim,
            witness: Some(witness),
            evidence: None,
        });
        self.revision += 1;
        self.next_id = next_id;
        Ok(id)
    }
    pub fn retain_range(
        &mut self,
        id: u64,
        claim: &SourceRangeClaim,
        evidence: Vec<u8>,
    ) -> Result<()> {
        check_evidence(&evidence)?;
        let batch = self.batch(id)?;
        if batch.claim == *claim && batch.evidence.as_deref() == Some(evidence.as_slice()) {
            return Ok(());
        }
        if batch.claim != *claim || batch.witness.is_none() {
            return Err(Error::Binding);
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if tx.execute(
            "UPDATE range_metadata SET revision=revision+1 WHERE id=1 AND revision=?1",
            [self.revision],
        )? != 1
        {
            return Err(Error::Stale);
        }
        if tx.execute(
            "UPDATE range_batches SET witness=NULL,evidence=?1 WHERE id=?2 AND evidence IS NULL",
            params![evidence, sql_id(id)?],
        )? != 1
        {
            return Err(Error::Binding);
        }
        durable::commit(
            tx,
            self.durability.as_deref(),
            self.revision,
            &[sql_id(id)?],
            &[],
            None,
            Vec::new(),
        )?;
        let batch = self
            .batches
            .iter_mut()
            .find(|b| b.id == id)
            .ok_or(Error::Binding)?;
        batch.witness = None;
        batch.evidence = Some(evidence);
        self.revision += 1;
        Ok(())
    }
    pub fn aggregation(&self, ids: &[u64]) -> Result<SourceRangeClaim> {
        let first = ids.first().ok_or(Error::Binding)?;
        let start = self
            .batches
            .iter()
            .position(|b| b.id == *first)
            .ok_or(Error::Binding)?;
        let end = start.checked_add(ids.len()).ok_or(Error::Binding)?;
        let batches = self.batches.get(start..end).ok_or(Error::Binding)?;
        if batches
            .iter()
            .zip(ids)
            .any(|(b, id)| b.id != *id || b.evidence.is_none())
        {
            return Err(Error::Binding);
        }
        Ok(SourceRangeClaim::aggregate(
            self.policy,
            &batches.iter().map(|b| b.claim.clone()).collect::<Vec<_>>(),
        )?)
    }
    pub fn retain_aggregate(
        &mut self,
        ids: &[u64],
        claim: &SourceRangeClaim,
        evidence: Vec<u8>,
    ) -> Result<()> {
        check_evidence(&evidence)?;
        if self.aggregation(ids)? != *claim {
            return Err(Error::Binding);
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if tx.execute(
            "UPDATE range_metadata SET revision=revision+1 WHERE id=1 AND revision=?1",
            [self.revision],
        )? != 1
        {
            return Err(Error::Stale);
        }
        for id in ids {
            tx.execute("DELETE FROM range_batches WHERE id=?1", [sql_id(*id)?])?;
        }
        tx.execute(
            "INSERT INTO range_batches VALUES(?1,?2,NULL,?3)",
            params![sql_id(ids[0])?, claim.to_bytes()?, evidence],
        )?;
        let removed = ids
            .iter()
            .map(|id| sql_id(*id))
            .collect::<Result<Vec<_>>>()?;
        durable::commit(
            tx,
            self.durability.as_deref(),
            self.revision,
            &[sql_id(ids[0])?],
            &removed,
            None,
            Vec::new(),
        )?;
        let start = self
            .batches
            .iter()
            .position(|b| b.id == ids[0])
            .ok_or(Error::Binding)?;
        self.batches.splice(
            start..start + ids.len(),
            [RangeBatch {
                id: ids[0],
                claim: claim.clone(),
                witness: None,
                evidence: Some(evidence),
            }],
        );
        self.revision += 1;
        Ok(())
    }
    pub fn retain_prefix(&mut self, claim: &SourceReceiptClaim, evidence: Vec<u8>) -> Result<()> {
        check_evidence(&evidence)?;
        check_claim(self.policy, claim)?;
        let through = if *claim == self.base {
            if self.evidence.as_deref() == Some(evidence.as_slice()) {
                return Ok(());
            }
            if self.evidence.is_some() {
                return Err(Error::Binding);
            }
            0
        } else {
            self.batches
                .iter()
                .position(|b| b.claim.end == *claim)
                .ok_or(Error::Binding)?
                + 1
        };
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if tx.execute("UPDATE range_metadata SET revision=revision+1,base=?1,evidence=?2 WHERE id=1 AND revision=?3",
            params![claim.to_bytes()?, evidence, self.revision])? != 1 { return Err(Error::Stale); }
        if through > 0 {
            tx.execute(
                "DELETE FROM range_batches WHERE id<=?1",
                [sql_id(self.batches[through - 1].id)?],
            )?;
        }
        let removed = if through > 0 {
            Some(sql_id(self.batches[through - 1].id)?)
        } else {
            None
        };
        durable::commit(
            tx,
            self.durability.as_deref(),
            self.revision,
            &[],
            &[],
            removed,
            Vec::new(),
        )?;
        self.batches.drain(..through);
        self.base = claim.clone();
        self.evidence = Some(evidence);
        self.revision += 1;
        Ok(())
    }
}
fn check_evidence(bytes: &[u8]) -> Result<()> {
    if bytes.is_empty() || bytes.len() > MAX_EVIDENCE {
        return Err(Error::Binding);
    }
    Ok(())
}
fn check_claim(policy: SourcePolicy, claim: &SourceReceiptClaim) -> Result<()> {
    claim.to_bytes()?;
    if claim.program != policy.program
        || claim.policy != policy.id()?
        || claim.genesis
            != crate::bootstrap::genesis_hash(policy.network)
                .map_err(super::TransitionError::from)?
        || claim.index.context.start() != policy.index_start
    {
        return Err(Error::Binding);
    }
    Ok(())
}

fn sql_id(id: u64) -> Result<i64> {
    i64::try_from(id).map_err(|_| Error::Binding)
}
