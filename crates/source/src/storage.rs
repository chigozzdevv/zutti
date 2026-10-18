use crate::{
    CandidateIndex, IndexSnapshot, Observation,
    access_index::HistoryIndex,
    authenticated::AuthenticatedMap,
    bootstrap::GenesisBlock,
    proof::{SourceInput, SourcePolicy, SourceReceiptClaim, SourceWitness},
    spends::CandidateSpends,
    state::{SourceCandidate, TransitionError},
    transaction::CandidateLedger,
    transition::{MAX_WITNESS, StateSnapshot, TransitionWitness},
};
use rusqlite::{Connection, OpenFlags, TransactionBehavior, params};
use std::{collections::BTreeSet, fs::OpenOptions, path::Path, time::Duration};
use zutti_protocol::{Digest, encoding::Canonical};

const APPLICATION: i32 = 0x5a555454;
const MAX_EVIDENCE: usize = 16 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Encoding(#[from] zutti_protocol::Error),
    #[error(transparent)]
    Source(#[from] TransitionError),
    #[error("source database does not match the configured policy or authenticated state")]
    Binding,
    #[error("source database changed; reopen before continuing")]
    Stale,
    #[error("the pending source batch requires evidence before advancing")]
    Pending,
}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
struct Stored {
    revision: i64,
    encoded_policy: Vec<u8>,
    state: Vec<u8>,
    pending: Option<Vec<u8>>,
    evidence: Option<Vec<u8>>,
}

#[derive(Debug)]
pub struct SourceStore {
    connection: Connection,
    policy: SourcePolicy,
    candidate: SourceCandidate,
    pending: Option<SourceWitness>,
    evidence: Option<Vec<u8>>,
    revision: i64,
}

impl SourceStore {
    pub fn create(path: &Path, genesis: Vec<u8>, policy: SourcePolicy) -> Result<Self> {
        let parsed =
            GenesisBlock::parse(&genesis, policy.network).map_err(TransitionError::from)?;
        let candidate = SourceCandidate::from_genesis(&parsed, policy.id()?, policy.index_start)?;
        let pending = SourceWitness {
            policy,
            input: SourceInput::Genesis(genesis),
        };
        let snapshot = candidate.state_snapshot()?;
        if pending.evaluate()?.state != snapshot.commitment()? {
            return Err(Error::Binding);
        }
        let file = OpenOptions::new().write(true).create_new(true).open(path)?;
        file.sync_all()?;
        let mut connection = connect(path)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(
            "PRAGMA application_id=1515541588;
            PRAGMA user_version=1;
            CREATE TABLE metadata (id INTEGER PRIMARY KEY CHECK(id=1), revision INTEGER NOT NULL,
                policy BLOB NOT NULL, state BLOB NOT NULL, pending BLOB, evidence BLOB);
            CREATE TABLE leaves (tree INTEGER NOT NULL CHECK(tree BETWEEN 0 AND 3),
                key BLOB NOT NULL CHECK(length(key)=32), value BLOB NOT NULL,
                PRIMARY KEY(tree,key)) WITHOUT ROWID;",
        )?;
        for (tree, map) in [
            (0, &candidate.ledger.outputs),
            (1, &candidate.spends.state),
            (3, &candidate.archive),
        ] {
            for (key, value) in map.stored_values().map_err(TransitionError::from)? {
                put(&transaction, tree, *key, Some(value))?;
            }
        }
        if let HistoryIndex::Full { index, .. } = &candidate.index {
            for (key, height) in index.stored_entries() {
                put(&transaction, 2, *key, Some(&height.to_le_bytes()))?;
            }
        } else {
            return Err(Error::Binding);
        }
        transaction.execute(
            "INSERT INTO metadata VALUES(1,0,?1,?2,?3,NULL)",
            params![
                policy.to_bytes()?,
                snapshot.to_bytes()?,
                pending.to_bytes()?
            ],
        )?;
        transaction.commit()?;
        std::fs::File::open(
            path.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )?
        .sync_all()?;
        Ok(Self {
            connection,
            policy,
            candidate,
            pending: Some(pending),
            evidence: None,
            revision: 0,
        })
    }

    pub fn open(path: &Path, policy: SourcePolicy) -> Result<Self> {
        let mut connection = connect(path)?;
        if connection.pragma_query_value(None, "application_id", |r| r.get::<_, i32>(0))?
            != APPLICATION
            || connection.pragma_query_value(None, "user_version", |r| r.get::<_, i32>(0))? != 1
        {
            return Err(Error::Binding);
        }
        let transaction = connection.transaction()?;
        let Stored {
            revision,
            encoded_policy,
            state,
            pending,
            evidence,
        } = transaction.query_row(
            "SELECT revision,policy,state,pending,evidence FROM metadata WHERE id=1",
            [],
            |row| {
                Ok(Stored {
                    revision: row.get(0)?,
                    encoded_policy: row.get(1)?,
                    state: row.get(2)?,
                    pending: row.get(3)?,
                    evidence: row.get(4)?,
                })
            },
        )?;
        if encoded_policy != policy.to_bytes()?
            || state.len() > MAX_WITNESS
            || pending.as_ref().is_some_and(|p| p.len() > MAX_WITNESS)
            || evidence
                .as_ref()
                .is_some_and(|e| e.is_empty() || e.len() > MAX_EVIDENCE)
        {
            return Err(Error::Binding);
        }
        let state = StateSnapshot::from_bytes(&state)?;
        if state.headers.network() != policy.network
            || state.index.context.rules() != policy.id()?
            || state.index.context.start() != policy.index_start
        {
            return Err(Error::Binding);
        }
        let ledger_map = load_map(
            &transaction,
            0,
            crate::transaction::ledger_context(),
            state.ledger_root,
        )?;
        if ledger_map.len().map(|n| n as u64) != Some(state.ledger_count) {
            return Err(Error::Binding);
        }
        let spends_map = load_map(&transaction, 1, crate::spends::context(), state.spends_root)?;
        let archive = load_map(
            &transaction,
            3,
            crate::checkpoint::context(state.index.context.genesis(), policy.id()?),
            state.archive_root,
        )?;
        let mut index = CandidateIndex::new(state.index.context);
        for_each_leaf(&transaction, 2, |key, value| {
            let height = u32::from_le_bytes(value.try_into().map_err(|_| Error::Binding)?);
            index
                .restore_entry(key, height, state.index.height)
                .map_err(TransitionError::from)?;
            Ok(())
        })?;
        index
            .restore_height(state.index)
            .map_err(TransitionError::from)?;
        let candidate = SourceCandidate {
            headers: state.headers.clone(),
            trees: state.trees.clone(),
            history: state.history.clone(),
            index: HistoryIndex::Full {
                index: Box::new(index),
                recorded: None,
            },
            ledger: CandidateLedger {
                outputs: ledger_map,
                count: state.ledger_count,
            },
            spends: CandidateSpends {
                state: spends_map,
                counts: state.nullifier_counts,
                balances: state.pool_balances,
            },
            monetary: state.monetary.clone(),
            archive,
        };
        let pending = pending
            .map(|bytes| SourceWitness::from_bytes(&bytes))
            .transpose()?;
        if let Some(witness) = &pending {
            if witness.policy != policy || witness.evaluate()?.state != state.commitment()? {
                return Err(Error::Binding);
            }
        } else if evidence.is_none() {
            return Err(Error::Binding);
        }
        transaction.commit()?;
        Ok(Self {
            connection,
            policy,
            candidate,
            pending,
            evidence,
            revision,
        })
    }

    pub const fn policy(&self) -> SourcePolicy {
        self.policy
    }
    pub const fn candidate(&self) -> &SourceCandidate {
        &self.candidate
    }
    pub const fn pending(&self) -> Option<&SourceWitness> {
        self.pending.as_ref()
    }
    pub fn evidence(&self) -> Option<&[u8]> {
        self.evidence.as_deref()
    }

    pub fn advance(&mut self, blocks: Vec<Vec<u8>>) -> Result<()> {
        if self.pending.is_some() {
            return Err(Error::Pending);
        }
        let previous =
            SourceReceiptClaim::from_state(self.policy, &self.candidate.state_snapshot()?)?;
        let mut next = self.candidate.clone();
        let transition = next.record_transition(blocks)?;
        let snapshot = next.state_snapshot()?;
        let pending = SourceWitness {
            policy: self.policy,
            input: SourceInput::Advance {
                previous: Box::new(previous),
                transition: Box::new(transition),
            },
        };
        let pending_bytes = pending.to_bytes()?;
        if pending_bytes.len() > MAX_WITNESS {
            return Err(Error::Binding);
        }
        let SourceInput::Advance { transition, .. } = &pending.input else {
            return Err(Error::Binding);
        };
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute("UPDATE metadata SET revision=revision+1,state=?1,pending=?2 WHERE id=1 AND revision=?3 AND pending IS NULL",
            params![snapshot.to_bytes()?, pending_bytes, self.revision])?;
        if changed != 1 {
            return Err(Error::Stale);
        }
        persist_delta(&transaction, &next, transition)?;
        transaction.commit()?;
        self.candidate = next;
        self.pending = Some(pending);
        self.revision += 1;
        Ok(())
    }

    pub fn retain_evidence(&mut self, state: Digest, evidence: Vec<u8>) -> Result<()> {
        if self.pending.is_none()
            || state != self.candidate.state_snapshot()?.commitment()?
            || evidence.is_empty()
            || evidence.len() > MAX_EVIDENCE
        {
            return Err(Error::Binding);
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if transaction.execute("UPDATE metadata SET revision=revision+1,pending=NULL,evidence=?1 WHERE id=1 AND revision=?2 AND pending IS NOT NULL", params![evidence, self.revision])? != 1 { return Err(Error::Stale); }
        transaction.commit()?;
        self.pending = None;
        self.evidence = Some(evidence);
        self.revision += 1;
        Ok(())
    }
}

fn connect(path: &Path) -> Result<Connection> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    connection.pragma_update(None, "trusted_schema", false)?;
    Ok(connection)
}
fn for_each_leaf(
    connection: &Connection,
    tree: u8,
    mut visit: impl FnMut(Digest, &[u8]) -> Result<()>,
) -> Result<()> {
    let mut statement =
        connection.prepare("SELECT key,value FROM leaves WHERE tree=?1 ORDER BY key")?;
    let mut rows = statement.query([tree])?;
    while let Some(row) = rows.next()? {
        let key = row.get_ref(0)?.as_blob().map_err(|_| Error::Binding)?;
        let value = row.get_ref(1)?.as_blob().map_err(|_| Error::Binding)?;
        visit(Digest(key.try_into().map_err(|_| Error::Binding)?), value)?;
    }
    Ok(())
}
fn load_map(
    connection: &Connection,
    tree: u8,
    context: Digest,
    expected: Digest,
) -> Result<AuthenticatedMap> {
    let mut map = AuthenticatedMap::new(context);
    for_each_leaf(connection, tree, |key, value| {
        map.set(key, Some(value.to_vec()))
            .map_err(TransitionError::from)?;
        Ok(())
    })?;
    if map.root() != expected {
        return Err(Error::Binding);
    }
    Ok(map)
}
fn put(connection: &Connection, tree: u8, key: Digest, value: Option<&[u8]>) -> Result<()> {
    match value {
        Some(value) => {
            connection.execute("INSERT INTO leaves VALUES(?1,?2,?3) ON CONFLICT(tree,key) DO UPDATE SET value=excluded.value", params![tree,key.0.as_slice(),value])?;
        }
        None => {
            connection.execute(
                "DELETE FROM leaves WHERE tree=?1 AND key=?2",
                params![tree, key.0.as_slice()],
            )?;
        }
    }
    Ok(())
}
fn persist_delta(
    connection: &Connection,
    source: &SourceCandidate,
    witness: &TransitionWitness,
) -> Result<()> {
    for (tree, map, transcript) in [
        (0, &source.ledger.outputs, &witness.ledger),
        (1, &source.spends.state, &witness.spends),
        (3, &source.archive, &witness.archive),
    ] {
        let keys = transcript
            .accesses()
            .iter()
            .map(|a| a.key())
            .collect::<BTreeSet<_>>();
        for key in keys {
            put(
                connection,
                tree,
                key,
                map.get(key).map_err(TransitionError::from)?.as_deref(),
            )?;
        }
    }
    let snapshot: IndexSnapshot = source.index.snapshot();
    for key in witness
        .index
        .iter()
        .map(|p| p.key())
        .collect::<BTreeSet<_>>()
    {
        let proof = source.index.prove(key).map_err(TransitionError::from)?;
        match proof.verify(snapshot, key).map_err(TransitionError::from)? {
            Observation::Present(height) => put(connection, 2, key, Some(&height.to_le_bytes()))?,
            Observation::Absent => return Err(Error::Binding),
        }
    }
    Ok(())
}
