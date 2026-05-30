use std::fmt::Debug;
use std::io;
use std::marker::PhantomData;
use std::ops::RangeBounds;

use byteorder::BigEndian;
use byteorder::ReadBytesExt;
use byteorder::WriteBytesExt;
use fjall::Database;
use fjall::Keyspace;
use fjall::KeyspaceCreateOptions;
use fjall::PersistMode;
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

use crate::log_store::meta::StoreMeta;

#[derive(Clone)]
pub struct FjallLogStore<C>
where C: RaftTypeConfig
{
    db: Database,
    _p: PhantomData<C>,
}

impl<C> FjallLogStore<C>
where C: RaftTypeConfig
{
    pub fn new(db: Database) -> Self {
        db.keyspace("meta", KeyspaceCreateOptions::default).expect("keyspace `meta` not found");
        db.keyspace("logs", KeyspaceCreateOptions::default).expect("keyspace `logs` not found");

        Self { db, _p: PhantomData }
    }

    fn keyspace_meta(&self) -> Keyspace {
        self.db.keyspace("meta", KeyspaceCreateOptions::default).unwrap()
    }

    fn keyspace_logs(&self) -> Keyspace {
        self.db.keyspace("logs", KeyspaceCreateOptions::default).unwrap()
    }

    fn get_meta<M: StoreMeta<C>>(&self) -> Result<Option<M::Value>, io::Error> {
        let val = self.keyspace_meta().get(M::KEY).map_err(|e| io::Error::other(e.to_string()))?;
        let Some(val) = val else {
            return Ok(None);
        };

        let t = serde_json::from_slice(val.as_ref()).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        Ok(Some(t))
    }

    fn put_meta<M: StoreMeta<C>>(&self, val: &M::Value) -> Result<(), io::Error> {
        let val = serde_json::to_vec(val).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        self.keyspace_meta().insert(M::KEY, val).map_err(|e| io::Error::other(e.to_string()))
    }
}

impl<C> RaftLogReader<C> for FjallLogStore<C>
where C: RaftTypeConfig
{
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + OptionalSend>(
        &mut self,
        range: RB,
    ) -> Result<Vec<C::Entry>, io::Error> {
        let start = match range.start_bound() {
            std::ops::Bound::Included(x) => id_to_bin(*x),
            std::ops::Bound::Excluded(x) => id_to_bin(*x + 1),
            std::ops::Bound::Unbounded => id_to_bin(0),
        };

        let mut res = Vec::new();

        for item_res in self.keyspace_logs().range(start..) {
            let (id, val) = item_res.into_inner().map_err(read_logs_err)?;

            let id = bin_to_id(id.as_ref());
            if !range.contains(&id) {
                break;
            }

            let entry: EntryOf<C> = serde_json::from_slice(val.as_ref()).map_err(read_logs_err)?;

            assert_eq!(id, entry.index());

            res.push(entry);
        }
        Ok(res)
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
        let last = self.keyspace_logs().iter().last();

        let last_log_id = match last {
            None => None,
            Some(res) => {
                let (_log_index, entry) = res.into_inner().map_err(read_logs_err)?;
                let ent = serde_json::from_slice::<EntryOf<C>>(entry.as_ref()).map_err(read_logs_err)?;
                Some(ent.log_id())
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

    async fn save_vote(&mut self, vote: &VoteOf<C>) -> Result<(), io::Error> {
        self.put_meta::<meta::Vote>(vote)?;

        // Vote must be persisted to disk before returning.
        let db = self.db.clone();
        C::spawn_blocking(move || db.persist(PersistMode::SyncAll).map_err(|e| io::Error::other(e.to_string())))
            .await??;

        Ok(())
    }

    async fn append<I>(&mut self, entries: I, callback: IOFlushed<C>) -> Result<(), io::Error>
    where I: IntoIterator<Item = EntryOf<C>> + Send {
        for entry in entries {
            let id = id_to_bin(entry.index());
            self.keyspace_logs()
                .insert(
                    id,
                    serde_json::to_vec(&entry).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?,
                )
                .map_err(|e| io::Error::other(e.to_string()))?;
        }

        let db = self.db.clone();
        std::thread::spawn(move || {
            let res = db.persist(PersistMode::SyncAll).map_err(|e| io::Error::other(e.to_string()));
            callback.io_completed(res)
        });

        Ok(())
    }

    async fn truncate_after(&mut self, last_log_id: Option<LogIdOf<C>>) -> Result<(), io::Error> {
        tracing::debug!("truncate_after: ({:?}, +∞)", last_log_id);

        let start_index = match last_log_id {
            Some(log_id) => log_id.index() + 1,
            None => 0,
        };

        // TODO(ariesdevil): using remove_range instead when
        // this pr merged https://github.com/fjall-rs/lsm-tree/pull/242
        for k in start_index..10_000 {
            self.keyspace_logs().remove(id_to_bin(k)).map_err(|e| io::Error::other(e.to_string()))?;
        }

        Ok(())
    }

    async fn purge(&mut self, log_id: LogIdOf<C>) -> Result<(), io::Error> {
        tracing::debug!("delete_log: [0, {:?}]", log_id);

        // Write the last-purged log id before purging the logs.
        // The logs at and before last-purged log id will be ignored by openraft.
        // Therefore, there is no need to do it in a transaction.
        self.put_meta::<meta::LastPurged>(&log_id)?;

        // TODO(ariesdevil): using remove_range instead when
        // this pr merged https://github.com/fjall-rs/lsm-tree/pull/242
        for k in 0..log_id.index() + 1 {
            self.keyspace_logs().remove(id_to_bin(k)).map_err(|e| io::Error::other(e.to_string()))?;
        }

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
        /// The key used to store in fjall
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

fn read_logs_err(e: impl std::error::Error + 'static) -> io::Error {
    io::Error::other(e.to_string())
}
