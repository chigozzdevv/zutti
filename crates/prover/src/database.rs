use mongodb::{
    bson::{self, Bson, Document, doc, spec::BinarySubtype},
    options::{Acknowledgment, DatabaseOptions, ReadConcern, WriteConcern},
    sync::{Client, Collection, Database},
};
use sha2::{Digest as _, Sha256};
use std::{
    fmt,
    fs::{File, OpenOptions},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::Path,
};
use zutti_protocol::{Digest, encoding::Canonical};
use zutti_source::{
    proof::SourcePolicy,
    storage::{
        Error, RangeLimits, RangeStore, Result,
        durable::{
            LeafRecord, RangeArchive, RangeCommit, RangeDurability, RangeMetadata, RangeRecord,
        },
    },
    transition::MAX_WITNESS,
};

const CHUNK: usize = 512 * 1024;
const MAX_EVIDENCE: usize = 16 * 1024 * 1024;
#[derive(Debug)]
struct MongoLease {
    database: MongoDatabase,
    _lock: File,
}
impl RangeDurability for MongoLease {
    fn commit(&self, update: &RangeCommit) -> Result<()> {
        self.database.commit(update)
    }
}
#[derive(Clone)]
pub struct MongoDatabase {
    client: Client,
    database: Database,
    store: String,
}
impl fmt::Debug for MongoDatabase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MongoDatabase")
            .field("store", &self.store)
            .finish_non_exhaustive()
    }
}
impl MongoDatabase {
    pub fn connect(uri: &str, database: &str, store: &str) -> Result<Self> {
        if !name(database) || !name(store) {
            return Err(Error::Binding);
        }
        let client = db(Client::with_uri_str(uri))?;
        let options = DatabaseOptions::builder()
            .read_concern(ReadConcern::majority())
            .write_concern(concern())
            .build();
        let database = client.database_with_options(database, options);
        let hello = db(database.run_command(doc! {"hello":1}).run())?;
        if hello.get_str("setName").is_err()
            || !hello.get_bool("isWritablePrimary").unwrap_or(false)
        {
            return Err(Error::Persistence);
        }
        Ok(Self {
            client,
            database,
            store: store.to_owned(),
        })
    }
    fn collection(&self, name: &str) -> Collection<Document> {
        self.database.collection(name)
    }
    pub fn archive(&self) -> Result<RangeArchive> {
        let doc = db(self
            .collection("source_metadata")
            .find_one(doc! {"_id":&self.store,"format":2})
            .run())?
        .ok_or(Error::Binding)?;
        let metadata = self.decode_metadata(&doc)?;
        let cursor = db(self
            .collection("source_batches")
            .find(doc! {"store":&self.store})
            .sort(doc! {"batch":1})
            .run())?;
        let mut batches = Vec::new();
        for row in cursor {
            if batches.len() >= 64 {
                return Err(Error::QueueFull);
            }
            batches.push(self.decode_record(&db(row)?)?);
        }
        let after = db(self
            .collection("source_metadata")
            .find_one(doc! {"_id":&self.store,"format":2})
            .run())?
        .ok_or(Error::Binding)?;
        if after.get_i64("revision").ok() != Some(metadata.revision) {
            return Err(Error::Stale);
        }
        Ok(RangeArchive { metadata, batches })
    }
    pub fn initialize(&self, store: &RangeStore) -> Result<()> {
        let archive = store.archive()?;
        if db(self
            .collection("source_metadata")
            .find_one(doc! {"_id":&self.store})
            .run())?
        .is_some()
        {
            if self.archive()? != archive {
                return Err(Error::Binding);
            }
            return self.verify_leaves(store);
        }
        let metadata = self.encode_metadata(&archive.metadata)?;
        let records = archive
            .batches
            .iter()
            .map(|r| self.encode_record(r))
            .collect::<Result<Vec<_>>>()?;
        let mut session = db(self.client.start_session().run())?;
        db(session
            .start_transaction()
            .write_concern(concern())
            .read_concern(ReadConcern::majority())
            .run())?;
        db(self
            .collection("source_metadata")
            .insert_one(metadata)
            .session(&mut session)
            .run())?;
        for record in records {
            db(self
                .collection("source_batches")
                .insert_one(record)
                .session(&mut session)
                .run())?;
        }
        let mut leaves = Vec::new();
        let mut leaf_bytes = 0usize;
        store.for_each_leaf(&mut |record| {
            leaf_bytes += record.value.len();
            leaves.push(self.encode_leaf(&record)?);
            if leaves.len() >= 128 || leaf_bytes >= 8 * 1024 * 1024 {
                db(self
                    .collection("source_leaves")
                    .insert_many(std::mem::take(&mut leaves))
                    .session(&mut session)
                    .run())?;
                leaf_bytes = 0;
            }
            Ok(())
        })?;
        if !leaves.is_empty() {
            db(self
                .collection("source_leaves")
                .insert_many(leaves)
                .session(&mut session)
                .run())?;
        }
        db(session.commit_transaction().run())?;
        Ok(())
    }
    pub fn open(
        &self,
        cache: &Path,
        policy: SourcePolicy,
        limits: RangeLimits,
    ) -> Result<RangeStore> {
        let parent = cache
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or(Error::Binding)?;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
        let lock_path = cache.with_extension("lock");
        if std::fs::symlink_metadata(&lock_path).is_ok_and(|m| !m.is_file()) {
            return Err(Error::Binding);
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(lock_path)?;
        lock.try_lock().map_err(|_| Error::Stale)?;
        let archive = self.archive()?;
        if archive.metadata.policy != policy.to_bytes()? {
            return Err(Error::Binding);
        }
        let cached = if cache.exists() {
            match RangeStore::open(cache, policy, limits) {
                Ok(store) if store.archive()? == archive => Some(store),
                _ => {
                    quarantine(cache)?;
                    None
                }
            }
        } else {
            None
        };
        let mut store = match cached {
            Some(store) => store,
            None => RangeStore::restore(cache, &archive, policy, limits, |put| {
                self.for_each_leaf(put)
            })?,
        };
        if self.archive()? != archive {
            return Err(Error::Stale);
        }
        store.attach_durability(Box::new(MongoLease {
            database: self.clone(),
            _lock: lock,
        }));
        Ok(store)
    }
    pub fn for_each_leaf(&self, visit: &mut dyn FnMut(LeafRecord) -> Result<()>) -> Result<()> {
        let cursor = db(self
            .collection("source_leaves")
            .find(doc! {"store":&self.store})
            .batch_size(512)
            .run())?;
        for item in cursor {
            let row = db(item)?;
            let tree = row.get_i32("tree").map_err(|_| Error::Binding)?;
            let key = bytes(&row, "key", 32)?;
            if !(0..=3).contains(&tree) || key.len() != 32 {
                return Err(Error::Binding);
            }
            let value = bytes(&row, "value", 4 * 1024 * 1024)?;
            visit(LeafRecord {
                tree: tree as u8,
                key: Digest(key.try_into().map_err(|_| Error::Binding)?),
                value,
            })?;
        }
        Ok(())
    }
    fn verify_leaves(&self, store: &RangeStore) -> Result<()> {
        let mut count = 0u64;
        store.for_each_leaf(&mut |leaf| {
            let doc = db(self
                .collection("source_leaves")
                .find_one(doc! {"_id":self.leaf_id(leaf.tree,leaf.key)})
                .run())?
            .ok_or(Error::Binding)?;
            if doc != self.encode_leaf(&leaf)? {
                return Err(Error::Binding);
            }
            count += 1;
            Ok(())
        })?;
        if db(self
            .collection("source_leaves")
            .count_documents(doc! {"store":&self.store})
            .run())?
            != count
        {
            return Err(Error::Binding);
        }
        Ok(())
    }
    fn leaf_id(&self, tree: u8, key: Digest) -> String {
        format!("{}:{tree}:{key}", self.store)
    }
    fn encode_leaf(&self, row: &LeafRecord) -> Result<Document> {
        if row.tree > 3 || row.value.len() > 4 * 1024 * 1024 {
            return Err(Error::Binding);
        }
        Ok(
            doc! {"_id":self.leaf_id(row.tree,row.key),"store":&self.store,"tree":i32::from(row.tree),"key":binary(&row.key.0),"value":binary(&row.value)},
        )
    }
    fn encode_metadata(&self, m: &RangeMetadata) -> Result<Document> {
        if m.revision < 0 || m.next_id < 1 {
            return Err(Error::Binding);
        }
        Ok(
            doc! {"_id":&self.store,"format":2,"revision":m.revision,"policy":binary(&m.policy),"state":self.put_blob(&m.state)?,"genesis":self.put_blob(&m.genesis)?,"base":binary(&m.base),"evidence":self.optional_blob(m.evidence.as_deref())?,"next_id":m.next_id},
        )
    }
    fn decode_metadata(&self, m: &Document) -> Result<RangeMetadata> {
        Ok(RangeMetadata {
            revision: m.get_i64("revision").map_err(|_| Error::Binding)?,
            policy: bytes(m, "policy", 4096)?,
            state: self.get_blob(
                m.get_document("state").map_err(|_| Error::Binding)?,
                MAX_WITNESS,
            )?,
            genesis: self.get_blob(
                m.get_document("genesis").map_err(|_| Error::Binding)?,
                MAX_WITNESS,
            )?,
            base: bytes(m, "base", 4096)?,
            evidence: self.read_optional(m, "evidence", MAX_EVIDENCE)?,
            next_id: m.get_i64("next_id").map_err(|_| Error::Binding)?,
        })
    }
    fn encode_record(&self, r: &RangeRecord) -> Result<Document> {
        if r.id < 1 {
            return Err(Error::Binding);
        }
        Ok(
            doc! {"_id":format!("{}:{}",self.store,r.id),"store":&self.store,"batch":r.id,"claim":binary(&r.claim),"witness":self.optional_blob(r.witness.as_deref())?,"evidence":self.optional_blob(r.evidence.as_deref())?},
        )
    }
    fn decode_record(&self, r: &Document) -> Result<RangeRecord> {
        Ok(RangeRecord {
            id: r.get_i64("batch").map_err(|_| Error::Binding)?,
            claim: bytes(r, "claim", 8192)?,
            witness: self.read_optional(r, "witness", MAX_WITNESS)?,
            evidence: self.read_optional(r, "evidence", MAX_EVIDENCE)?,
        })
    }
    fn optional_blob(&self, bytes: Option<&[u8]>) -> Result<Bson> {
        bytes
            .map(|b| self.put_blob(b).map(Bson::Document))
            .transpose()
            .map(|v| v.unwrap_or(Bson::Null))
    }
    fn read_optional(&self, doc: &Document, key: &str, limit: usize) -> Result<Option<Vec<u8>>> {
        match doc.get(key) {
            Some(Bson::Null) => Ok(None),
            Some(Bson::Document(value)) => self.get_blob(value, limit).map(Some),
            _ => Err(Error::Binding),
        }
    }
    pub fn put_blob(&self, value: &[u8]) -> Result<Document> {
        if value.len() > MAX_WITNESS {
            return Err(Error::Binding);
        }
        let id = hex::encode(Sha256::digest(value));
        for (index, chunk) in value.chunks(CHUNK).enumerate() {
            let key = format!("{id}:{index}");
            db(self
                .collection("source_blobs")
                .update_one(
                    doc! {"_id":key},
                    doc! {"$setOnInsert":{"blob":&id,"chunk":index as i32,"bytes":binary(chunk)}},
                )
                .upsert(true)
                .run())?;
        }
        Ok(doc! {"digest":id,"size":value.len() as i64})
    }
    pub fn get_blob(&self, reference: &Document, limit: usize) -> Result<Vec<u8>> {
        let id = reference.get_str("digest").map_err(|_| Error::Binding)?;
        let size = reference.get_i64("size").map_err(|_| Error::Binding)?;
        if Digest::from_hex(id).is_err() || size < 0 || size as u64 > limit.min(MAX_WITNESS) as u64
        {
            return Err(Error::Binding);
        }
        let mut value = Vec::with_capacity(size as usize);
        let cursor = db(self
            .collection("source_blobs")
            .find(doc! {"blob":id})
            .sort(doc! {"chunk":1})
            .run())?;
        for (index, item) in cursor.enumerate() {
            let row = db(item)?;
            if row.get_i32("chunk").ok() != Some(index as i32) {
                return Err(Error::Binding);
            }
            let bytes = bytes(&row, "bytes", CHUNK)?;
            if bytes.is_empty()
                || value
                    .len()
                    .checked_add(bytes.len())
                    .is_none_or(|n| n > size as usize)
                || (bytes.len() != CHUNK && value.len() + bytes.len() != size as usize)
            {
                return Err(Error::Binding);
            }
            value.extend_from_slice(&bytes);
        }
        if value.len() != size as usize || hex::encode(Sha256::digest(&value)) != id {
            return Err(Error::Binding);
        }
        Ok(value)
    }
}
fn quarantine(cache: &Path) -> Result<()> {
    let parent = cache.parent().ok_or(Error::Binding)?;
    let dir = tempfile::Builder::new()
        .prefix("source-cache-recovery-")
        .tempdir_in(parent)?
        .keep();
    let filename = cache.file_name().ok_or(Error::Binding)?;
    for suffix in ["", "-wal", "-shm"] {
        let mut original = cache.as_os_str().to_os_string();
        original.push(suffix);
        if Path::new(&original).exists() {
            let mut retained = filename.to_os_string();
            retained.push(suffix);
            std::fs::rename(original, dir.join(retained))?;
        }
    }
    Ok(())
}
impl RangeDurability for MongoDatabase {
    fn commit(&self, update: &RangeCommit) -> Result<()> {
        if update.metadata.revision
            != update
                .expected_revision
                .checked_add(1)
                .ok_or(Error::Binding)?
        {
            return Err(Error::Binding);
        }
        let metadata = self.encode_metadata(&update.metadata)?;
        let records = update
            .batches
            .iter()
            .map(|r| self.encode_record(r))
            .collect::<Result<Vec<_>>>()?;
        let mut session = db(self.client.start_session().run())?;
        db(session
            .start_transaction()
            .write_concern(concern())
            .read_concern(ReadConcern::majority())
            .run())?;
        let result = db(self
            .collection("source_metadata")
            .replace_one(
                doc! {"_id":&self.store,"format":2,"revision":update.expected_revision},
                metadata,
            )
            .session(&mut session)
            .run())?;
        if result.matched_count != 1 {
            return Err(Error::Stale);
        }
        for id in &update.removed {
            db(self
                .collection("source_batches")
                .delete_one(doc! {"_id":format!("{}:{id}",self.store)})
                .session(&mut session)
                .run())?;
        }
        if let Some(through) = update.through {
            db(self
                .collection("source_batches")
                .delete_many(doc! {"store":&self.store,"batch":{"$lte":through}})
                .session(&mut session)
                .run())?;
        }
        for record in records {
            db(self
                .collection("source_batches")
                .replace_one(
                    doc! {"_id":record.get_str("_id").map_err(|_|Error::Binding)?},
                    &record,
                )
                .upsert(true)
                .session(&mut session)
                .run())?;
        }
        for leaf in &update.leaves {
            match &leaf.value {
                Some(value) => {
                    let record = self.encode_leaf(&LeafRecord {
                        tree: leaf.tree,
                        key: leaf.key,
                        value: value.clone(),
                    })?;
                    db(self
                        .collection("source_leaves")
                        .replace_one(doc! {"_id":self.leaf_id(leaf.tree,leaf.key)}, record)
                        .upsert(true)
                        .session(&mut session)
                        .run())?;
                }
                None => {
                    db(self
                        .collection("source_leaves")
                        .delete_one(doc! {"_id":self.leaf_id(leaf.tree,leaf.key)})
                        .session(&mut session)
                        .run())?;
                }
            }
        }
        db(session.commit_transaction().run())?;
        Ok(())
    }
}
fn db<T>(result: mongodb::error::Result<T>) -> Result<T> {
    result.map_err(|_| Error::Persistence)
}
fn binary(bytes: &[u8]) -> bson::Binary {
    bson::Binary {
        subtype: BinarySubtype::Generic,
        bytes: bytes.to_vec(),
    }
}
fn bytes(doc: &Document, key: &str, limit: usize) -> Result<Vec<u8>> {
    let value = doc.get_binary_generic(key).map_err(|_| Error::Binding)?;
    if value.len() > limit {
        return Err(Error::Binding);
    }
    Ok(value.clone())
}
fn concern() -> WriteConcern {
    WriteConcern::builder()
        .w(Acknowledgment::Majority)
        .journal(true)
        .build()
}
fn name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

#[cfg(test)]
#[path = "database/tests.rs"]
mod tests;
