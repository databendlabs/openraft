//! Provide a fjall-backed log storage implementation for examples.
//!
//! [`FjallLogStore`] keeps Raft log entries in the `logs` keyspace of a [`fjall::Database`],
//! keyed by the big-endian log index, and the vote and the last purged log id in the `meta`
//! keyspace.

#![deny(unused_crate_dependencies)]
#![deny(unused_qualifications)]

#[cfg(test)]
mod test;

use std::fmt::Debug;
use std::io;
use std::marker::PhantomData;
use std::ops::RangeBounds;
use std::path::Path;

use fjall::Database;
use fjall::Keyspace;
use fjall::KeyspaceCreateOptions;
use fjall::OwnedWriteBatch;
use fjall::PersistMode;
use fjall::Readable;
use meta::StoreMeta;
use openraft::LogState;
use openraft::OptionalSend;
use openraft::RaftLogReader;
use openraft::RaftTypeConfig;
use openraft::alias::EntryOf;
use openraft::alias::LogIdOf;
use openraft::alias::VoteOf;
use openraft::entry::RaftEntry;
use openraft::storage::IOFlushed;
use openraft::storage::RaftLogStorage;
use openraft::type_config::TypeConfigExt;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// A [`RaftLogStorage`] backed by a fjall [`Database`].
///
/// Every clone shares the same database, so the clone returned by
/// [`RaftLogStorage::get_log_reader`] reads every entry written through the store.
#[derive(Clone)]
pub struct FjallLogStore<C>
where C: RaftTypeConfig
{
    db: Database,

    /// The vote and the last purged log id.
    meta: Keyspace,

    /// Log entries, keyed by the big-endian log index so that key order is index order.
    logs: Keyspace,

    _p: PhantomData<C>,
}

impl<C> FjallLogStore<C>
where C: RaftTypeConfig
{
    /// Open a standalone fjall database at `db_path` and store the log in it.
    pub fn open<P>(db_path: P) -> Result<Self, io::Error>
    where P: AsRef<Path> {
        let db = Database::builder(db_path).open().map_err(io::Error::other)?;
        Self::new(db)
    }

    /// Store the log in the `meta` and `logs` keyspaces of `db`, creating them if absent.
    ///
    /// Other components, such as a state machine, may share `db` as long as they use other
    /// keyspaces.
    pub fn new(db: Database) -> Result<Self, io::Error> {
        let meta = db.keyspace("meta", KeyspaceCreateOptions::default).map_err(io::Error::other)?;
        let logs = db.keyspace("logs", KeyspaceCreateOptions::default).map_err(io::Error::other)?;

        Ok(Self {
            db,
            meta,
            logs,
            _p: PhantomData,
        })
    }

    /// Read a store metadata, or `None` if it has never been written.
    fn get_meta<M>(&self) -> Result<Option<M::Value>, io::Error>
    where M: StoreMeta<C> {
        let bytes = self.meta.get(M::KEY).map_err(io::Error::other)?;
        bytes.map(|bytes| deserialize(&bytes)).transpose()
    }

    /// Add a write of a store metadata to `batch`.
    fn put_meta<M>(&self, batch: &mut OwnedWriteBatch, value: &M::Value) -> Result<(), io::Error>
    where M: StoreMeta<C> {
        batch.insert(&self.meta, M::KEY, serialize(value)?);
        Ok(())
    }
}

impl<C> RaftLogReader<C> for FjallLogStore<C>
where C: RaftTypeConfig
{
    async fn try_get_log_entries<RB>(&mut self, range: RB) -> Result<Vec<C::Entry>, io::Error>
    where RB: RangeBounds<u64> + Clone + Debug + OptionalSend {
        let start = range.start_bound().map(|index| index_key(*index));
        let end = range.end_bound().map(|index| index_key(*index));

        // Scan a database snapshot, so that writes made during the scan are not seen.
        let mut entries = Vec::<EntryOf<C>>::new();
        for item in self.db.snapshot().range(&self.logs, (start, end)) {
            let (key, value) = item.into_inner().map_err(io::Error::other)?;
            let index = key_index(&key)?;
            let entry: EntryOf<C> = deserialize(&value)?;

            if entry.index() != index {
                return Err(invalid_data(format!(
                    "log entry {} is stored at index {}",
                    entry.log_id(),
                    index
                )));
            }

            if let Some(prev) = entries.last()
                && prev.index() + 1 != index
            {
                return Err(invalid_data(format!(
                    "hole in logs: index {} follows {}",
                    index,
                    prev.index()
                )));
            }

            entries.push(entry);
        }

        Ok(entries)
    }

    async fn read_vote(&mut self) -> Result<Option<VoteOf<C>>, io::Error> {
        self.get_meta::<meta::Vote>()
    }
}

impl<C> RaftLogStorage<C> for FjallLogStore<C>
where C: RaftTypeConfig
{
    type LogReader = Self;

    async fn get_log_state(&mut self) -> Result<LogState<C>, io::Error> {
        let last_purged_log_id = self.get_meta::<meta::LastPurged>()?;

        let last_log_id = match self.logs.last_key_value() {
            Some(item) => {
                let value = item.value().map_err(io::Error::other)?;
                Some(deserialize::<EntryOf<C>>(&value)?.log_id())
            }
            // Either no entry was written, or every entry is purged.
            None => last_purged_log_id.clone(),
        };

        Ok(LogState {
            last_purged_log_id,
            last_log_id,
        })
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.clone()
    }

    async fn save_vote(&mut self, vote: &VoteOf<C>) -> Result<(), io::Error> {
        // The vote must be on disk before returning: commit with fsync, off the async runtime.
        let mut batch = self.db.batch().durability(Some(PersistMode::SyncAll));
        self.put_meta::<meta::Vote>(&mut batch, vote)?;

        C::spawn_blocking(move || batch.commit()).await?.map_err(io::Error::other)
    }

    async fn append<I>(&mut self, entries: I, callback: IOFlushed<C>) -> Result<(), io::Error>
    where
        I: IntoIterator<Item = EntryOf<C>> + OptionalSend,
        I::IntoIter: OptionalSend,
    {
        let mut batch = self.db.batch();
        for entry in entries {
            batch.insert(&self.logs, index_key(entry.index()), serialize(&entry)?);
        }

        // Commit before returning: the entries must be readable when this method returns.
        batch.commit().map_err(io::Error::other)?;

        // Make the entries durable on another thread and report the result through `callback`.
        // All keyspaces of a database share one journal, and syncing it also persists every
        // earlier write, such as a preceding truncation.
        let db = self.db.clone();
        std::thread::spawn(move || {
            let res = db.persist(PersistMode::SyncAll).map_err(io::Error::other);
            callback.io_completed(res);
        });

        Ok(())
    }

    async fn truncate_after(&mut self, last_log_id: Option<LogIdOf<C>>) -> Result<(), io::Error> {
        tracing::debug!("truncate_after: ({:?}, +oo)", last_log_id);

        let start = last_log_id.map_or(0, |log_id| log_id.index() + 1);

        // fjall has no range delete, so every removed entry needs its own tombstone. Write them in
        // one atomic batch: removing entries one by one from `start` would leave a hole in the
        // logs if the process crashed halfway.
        let mut batch = self.db.batch();
        for item in self.logs.range(index_key(start)..) {
            batch.remove(&self.logs, item.key().map_err(io::Error::other)?);
        }
        batch.commit().map_err(io::Error::other)?;

        // Truncating does not need to be persisted: a crash may bring the removed entries back,
        // and Openraft truncates them again. The next `append()` syncs the journal, which
        // persists the truncation before the entries that replace them.
        Ok(())
    }

    async fn purge(&mut self, log_id: LogIdOf<C>) -> Result<(), io::Error> {
        tracing::debug!("purge: [0, {:?}]", log_id);

        // Every entry up to the previous purge point is removed already. Start after it, so that
        // the scan does not walk the tombstones left by earlier purges.
        let start = self.get_meta::<meta::LastPurged>()?.map_or(0, |purged| purged.index() + 1);
        let end = log_id.index();

        // Record the purge point and remove the entries in one atomic batch.
        let mut batch = self.db.batch();
        self.put_meta::<meta::LastPurged>(&mut batch, &log_id)?;
        if start <= end {
            for item in self.logs.range(index_key(start)..=index_key(end)) {
                batch.remove(&self.logs, item.key().map_err(io::Error::other)?);
            }
        }
        batch.commit().map_err(io::Error::other)?;

        // Purging does not need to be persisted: a crash that loses the batch brings back the
        // entries together with the previous purge point, and Openraft purges them again later.
        Ok(())
    }
}

/// Metadata of a raft-store.
///
/// In raft, except logs and state machine, the store also has to store several piece of metadata.
/// This sub mod defines the key-value pairs of these metadata.
mod meta {
    use openraft::RaftTypeConfig;
    use openraft::alias::LogIdOf;
    use openraft::alias::VoteOf;

    /// Defines metadata key and value
    pub(crate) trait StoreMeta<C>
    where C: RaftTypeConfig
    {
        /// The key used to store in the `meta` keyspace
        const KEY: &'static str;

        /// The type of the value to store
        type Value: serde::Serialize + serde::de::DeserializeOwned;
    }

    pub(crate) struct LastPurged {}
    pub(crate) struct Vote {}

    impl<C> StoreMeta<C> for LastPurged
    where C: RaftTypeConfig
    {
        const KEY: &'static str = "last_purged_log_id";
        type Value = LogIdOf<C>;
    }
    impl<C> StoreMeta<C> for Vote
    where C: RaftTypeConfig
    {
        const KEY: &'static str = "vote";
        type Value = VoteOf<C>;
    }
}

/// Encode a log index as a key: big-endian, so that the key order is the index order.
fn index_key(index: u64) -> [u8; 8] {
    index.to_be_bytes()
}

/// Decode the log index from a key written by [`index_key`].
fn key_index(key: &[u8]) -> Result<u64, io::Error> {
    let bytes = key.try_into().map_err(|_| invalid_data(format!("invalid log key: {:?}", key)))?;
    Ok(u64::from_be_bytes(bytes))
}

fn serialize<T>(value: &T) -> Result<Vec<u8>, io::Error>
where T: Serialize {
    serde_json::to_vec(value).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn deserialize<T>(bytes: &[u8]) -> Result<T, io::Error>
where T: DeserializeOwned {
    serde_json::from_slice(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn invalid_data(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
