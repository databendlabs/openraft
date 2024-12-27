use std::error::Error;
use std::fmt::Debug;
use std::marker::PhantomData;
use std::ops::RangeBounds;
use std::sync::Arc;

use byteorder::BigEndian;
use byteorder::ReadBytesExt;
use byteorder::WriteBytesExt;
use meta::StoreMeta;
use openraft::alias::EntryOf;
use openraft::alias::LogIdOf;
use openraft::alias::VoteOf;
use openraft::storage::IOFlushed;
use openraft::storage::RaftLogStorage;
use openraft::LogState;
use openraft::OptionalSend;
use openraft::RaftLogId;
use openraft::RaftLogReader;
use openraft::RaftTypeConfig;
use openraft::StorageError;
use rocksdb::ColumnFamily;
use rocksdb::Direction;
use rocksdb::DB;

#[derive(Debug, Clone)]
pub struct RocksLogStore<C>
where C: RaftTypeConfig
{
    db: Arc<DB>,
    _p: PhantomData<C>,
}

impl<C> RocksLogStore<C>
where C: RaftTypeConfig
{
    pub fn new(db: Arc<DB>) -> Self {
        db.cf_handle("meta").expect("column family `meta` not found");
        db.cf_handle("logs").expect("column family `logs` not found");

        Self {
            db,
            _p: Default::default(),
        }
    }

    fn cf_meta(&self) -> &ColumnFamily {
        self.db.cf_handle("meta").unwrap()
    }

    fn cf_logs(&self) -> &ColumnFamily {
        self.db.cf_handle("logs").unwrap()
    }

    /// Get a store metadata.
    ///
    /// It returns `None` if the store does not have such a metadata stored.
    fn get_meta<M: StoreMeta<C>>(&self) -> Result<Option<M::Value>, StorageError<C>> {
        let bytes = self.db.get_cf(self.cf_meta(), M::KEY).map_err(M::read_err)?;

        let Some(bytes) = bytes else {
            return Ok(None);
        };

        let t = serde_json::from_slice(&bytes).map_err(M::read_err)?;

        Ok(Some(t))
    }

    /// Save a store metadata.
    fn put_meta<M: StoreMeta<C>>(&self, value: &M::Value) -> Result<(), StorageError<C>> {
        let json_value = serde_json::to_vec(value).map_err(|e| M::write_err(value, e))?;

        self.db.put_cf(self.cf_meta(), M::KEY, json_value).map_err(|e| M::write_err(value, e))?;

        Ok(())
    }
}

impl<C> RaftLogReader<C> for RocksLogStore<C>
where C: RaftTypeConfig
{
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + OptionalSend>(
        &mut self,
        range: RB,
    ) -> Result<Vec<C::Entry>, StorageError<C>> {
        let start = match range.start_bound() {
            std::ops::Bound::Included(x) => id_to_bin(*x),
            std::ops::Bound::Excluded(x) => id_to_bin(*x + 1),
            std::ops::Bound::Unbounded => id_to_bin(0),
        };

        let mut res = Vec::new();

        let it = self.db.iterator_cf(self.cf_logs(), rocksdb::IteratorMode::From(&start, Direction::Forward));
        for item_res in it {
            let (id, val) = item_res.map_err(read_logs_err)?;

            let id = bin_to_id(&id);
            if !range.contains(&id) {
                break;
            }

            let entry: EntryOf<C> = serde_json::from_slice(&val).map_err(read_logs_err)?;

            assert_eq!(id, entry.get_log_id().index);

            res.push(entry);
        }
        Ok(res)
    }

    async fn read_vote(&mut self) -> Result<Option<VoteOf<C>>, StorageError<C>> {
        self.get_meta::<meta::Vote>()
    }
}

impl<C> RaftLogStorage<C> for RocksLogStore<C>
where C: RaftTypeConfig
{
    type LogReader = Self;

    async fn get_log_state(&mut self) -> Result<LogState<C>, StorageError<C>> {
        let last = self.db.iterator_cf(self.cf_logs(), rocksdb::IteratorMode::End).next();

        let last_log_id = match last {
            None => None,
            Some(res) => {
                let (_log_index, entry_bytes) = res.map_err(read_logs_err)?;
                let ent = serde_json::from_slice::<EntryOf<C>>(&entry_bytes).map_err(read_logs_err)?;
                Some(ent.get_log_id().clone())
            }
        };

        let last_purged_log_id = self.get_meta::<meta::LastPurged>()?;

        let last_log_id = match last_log_id {
            None => last_purged_log_id.clone(),
            Some(x) => Some(x),
        };

        Ok(LogState {
            last_purged_log_id,
            last_log_id,
        })
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.clone()
    }

    async fn save_vote(&mut self, vote: &VoteOf<C>) -> Result<(), StorageError<C>> {
        self.put_meta::<meta::Vote>(vote)?;
        self.db.flush_wal(true).map_err(|e| StorageError::write_vote(&e))?;
        Ok(())
    }

    async fn append<I>(&mut self, entries: I, callback: IOFlushed<C>) -> Result<(), StorageError<C>>
    where I: IntoIterator<Item = EntryOf<C>> + Send {
        for entry in entries {
            let id = id_to_bin(entry.get_log_id().index);
            assert_eq!(bin_to_id(&id), entry.get_log_id().index);
            self.db
                .put_cf(
                    self.cf_logs(),
                    id,
                    serde_json::to_vec(&entry).map_err(|e| StorageError::write_logs(&e))?,
                )
                .map_err(|e| StorageError::write_logs(&e))?;
        }

        self.db.flush_wal(true).map_err(|e| StorageError::write_logs(&e))?;

        // If there is error, the callback will be dropped.
        callback.io_completed(Ok(()));
        Ok(())
    }

    async fn truncate(&mut self, log_id: LogIdOf<C>) -> Result<(), StorageError<C>> {
        tracing::debug!("truncate: [{:?}, +oo)", log_id);

        let from = id_to_bin(log_id.index);
        let to = id_to_bin(0xff_ff_ff_ff_ff_ff_ff_ff);
        self.db.delete_range_cf(self.cf_logs(), &from, &to).map_err(|e| StorageError::write_logs(&e))?;

        self.db.flush_wal(true).map_err(|e| StorageError::write_logs(&e))?;
        Ok(())
    }

    async fn purge(&mut self, log_id: LogIdOf<C>) -> Result<(), StorageError<C>> {
        tracing::debug!("delete_log: [0, {:?}]", log_id);

        // Write the last-purged log id before purging the logs.
        // The logs at and before last-purged log id will be ignored by openraft.
        // Therefore, there is no need to do it in a transaction.
        self.put_meta::<meta::LastPurged>(&log_id)?;

        let from = id_to_bin(0);
        let to = id_to_bin(log_id.index + 1);
        self.db.delete_range_cf(self.cf_logs(), &from, &to).map_err(|e| StorageError::write_logs(&e))?;

        // Purging does not need to be persistent.
        Ok(())
    }
}

/// Metadata of a raft-store.
///
/// In raft, except logs and state machine, the store also has to store several piece of metadata.
/// This sub mod defines the key-value pairs of these metadata.
mod meta {
    use openraft::alias::LogIdOf;
    use openraft::alias::VoteOf;
    use openraft::AnyError;
    use openraft::ErrorSubject;
    use openraft::ErrorVerb;
    use openraft::RaftTypeConfig;
    use openraft::StorageError;

    /// Defines metadata key and value
    pub(crate) trait StoreMeta<C>
    where C: RaftTypeConfig
    {
        /// The key used to store in rocksdb
        const KEY: &'static str;

        /// The type of the value to store
        type Value: serde::Serialize + serde::de::DeserializeOwned;

        /// The subject this meta belongs to, and will be embedded into the returned storage error.
        fn subject(v: Option<&Self::Value>) -> ErrorSubject<C>;

        fn read_err(e: impl std::error::Error + 'static) -> StorageError<C> {
            StorageError::new(Self::subject(None), ErrorVerb::Read, AnyError::new(&e))
        }

        fn write_err(v: &Self::Value, e: impl std::error::Error + 'static) -> StorageError<C> {
            StorageError::new(Self::subject(Some(v)), ErrorVerb::Write, AnyError::new(&e))
        }
    }

    pub(crate) struct LastPurged {}
    pub(crate) struct Vote {}

    impl<C> StoreMeta<C> for LastPurged
    where C: RaftTypeConfig
    {
        const KEY: &'static str = "last_purged_log_id";
        type Value = LogIdOf<C>;

        fn subject(_v: Option<&Self::Value>) -> ErrorSubject<C> {
            ErrorSubject::Store
        }
    }
    impl<C> StoreMeta<C> for Vote
    where C: RaftTypeConfig
    {
        const KEY: &'static str = "vote";
        type Value = VoteOf<C>;

        fn subject(_v: Option<&Self::Value>) -> ErrorSubject<C> {
            ErrorSubject::Vote
        }
    }
}

/// converts an id to a byte vector for storing in the database.
/// Note that we're using big endian encoding to ensure correct sorting of keys
fn id_to_bin(id: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(8);
    buf.write_u64::<BigEndian>(id).unwrap();
    buf
}

fn bin_to_id(buf: &[u8]) -> u64 {
    (&buf[0..8]).read_u64::<BigEndian>().unwrap()
}

fn read_logs_err<C>(e: impl Error + 'static) -> StorageError<C>
where C: RaftTypeConfig {
    StorageError::read_logs(&e)
}
