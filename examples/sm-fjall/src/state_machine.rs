//! fjall-backed state machine implementation.

use std::collections::BTreeMap;
use std::io;
use std::io::Cursor;
use std::marker::PhantomData;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;

use fjall::Database;
use fjall::Keyspace;
use fjall::KeyspaceCreateOptions;
use fjall::PersistMode;
use fjall::Readable;
use futures::Stream;
use futures::TryStreamExt;
use openraft::EntryPayload;
use openraft::OptionalSend;
use openraft::RaftTypeConfig;
use openraft::alias::DefaultEntryOf;
use openraft::alias::EntryPayloadOf;
use openraft::alias::LogIdOf;
use openraft::alias::SnapshotMetaOf;
use openraft::alias::SnapshotOf;
use openraft::alias::StoredMembershipOf;
use openraft::entry::RaftEntry;
use openraft::storage::EntryResponder;
use openraft::storage::RaftStateMachine;
use openraft::type_config::TypeConfigExt;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::FjallSnapshotBuilder;

/// Key of the last applied log id in the `sm_meta` keyspace.
const LAST_APPLIED_LOG: &str = "last_applied_log";

/// Key of the last applied membership in the `sm_meta` keyspace.
const LAST_MEMBERSHIP: &str = "last_membership";

/// Key of the current snapshot's metadata in the `sm_snapshot` keyspace.
const SNAPSHOT_META: &str = "meta";

/// Key of the current snapshot's data in the `sm_snapshot` keyspace.
const SNAPSHOT_DATA: &str = "data";

/// State machine backed by a fjall [`Database`] for full persistence.
///
/// Application data lives in the `sm_data` keyspace, the last applied log id and membership in
/// `sm_meta`, and the current snapshot in `sm_snapshot`. Every clone shares the same database.
#[derive(Clone)]
pub struct FjallStateMachine<C>
where C: RaftTypeConfig
{
    db: Database,

    /// Application data: a key maps to its JSON-encoded [`types_kv::VersionedValue`].
    data: Keyspace,

    /// The last applied log id and the last applied membership.
    meta: Keyspace,

    /// Metadata and data of the current snapshot.
    snapshot: Keyspace,

    /// Serializes the writes to `sm_snapshot`, so that a snapshot built from an older view never
    /// replaces a newer one installed while it was being built.
    snapshot_lock: Arc<Mutex<()>>,

    _p: PhantomData<C>,
}

impl<C> FjallStateMachine<C>
where C: RaftTypeConfig
{
    /// Open a standalone fjall database at `db_path` and keep the state machine in it.
    ///
    /// The log store lives in the `log-fjall` crate and is opened separately.
    pub fn open<P>(db_path: P) -> Result<Self, io::Error>
    where P: AsRef<Path> {
        let db = Database::builder(db_path).open().map_err(io::Error::other)?;
        Self::new(db)
    }

    /// Keep the state machine in the `sm_data`, `sm_meta` and `sm_snapshot` keyspaces of `db`,
    /// creating them if absent.
    ///
    /// Other components, such as a log store, may share `db` as long as they use other keyspaces.
    pub fn new(db: Database) -> Result<Self, io::Error> {
        let data = db.keyspace("sm_data", KeyspaceCreateOptions::default).map_err(io::Error::other)?;
        let meta = db.keyspace("sm_meta", KeyspaceCreateOptions::default).map_err(io::Error::other)?;
        let snapshot = db.keyspace("sm_snapshot", KeyspaceCreateOptions::default).map_err(io::Error::other)?;

        Ok(Self {
            db,
            data,
            meta,
            snapshot,
            snapshot_lock: Arc::default(),
            _p: PhantomData,
        })
    }

    /// Read the value stored under `key`.
    pub fn get(&self, key: &str) -> Result<Option<types_kv::VersionedValue>, io::Error> {
        let bytes = self.data.get(key).map_err(io::Error::other)?;
        bytes.map(|bytes| deserialize(&bytes)).transpose()
    }

    /// Read the applied state in `view`, as the metadata of a snapshot of it.
    pub(crate) fn read_applied<R>(&self, view: &R) -> Result<SnapshotMetaOf<C>, io::Error>
    where R: Readable {
        let last_log_id = read(view, &self.meta, LAST_APPLIED_LOG)?;
        let last_membership = read(view, &self.meta, LAST_MEMBERSHIP)?.unwrap_or_default();

        Ok(SnapshotMetaOf::<C> {
            last_log_id,
            last_membership,
        })
    }

    /// Encode the application data in `view` as snapshot data.
    pub(crate) fn encode_data<R>(&self, view: &R) -> Result<Vec<u8>, io::Error>
    where R: Readable {
        let mut kvs = BTreeMap::<String, types_kv::VersionedValue>::new();

        for item in view.iter(&self.data) {
            let (key, value) = item.into_inner().map_err(io::Error::other)?;
            let key = String::from_utf8(key.to_vec()).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            kvs.insert(key, deserialize(&value)?);
        }

        serialize(&kvs)
    }

    /// Store a built snapshot as the current one, unless a newer snapshot is stored already.
    pub(crate) fn save_built_snapshot(&self, meta: &SnapshotMetaOf<C>, data: &[u8]) -> Result<(), io::Error> {
        let _guard = self.snapshot_lock.lock().unwrap_or_else(PoisonError::into_inner);

        let current = read::<SnapshotMetaOf<C>, _>(&self.db.snapshot(), &self.snapshot, SNAPSHOT_META)?;
        if current.is_some_and(|current| current.last_log_id > meta.last_log_id) {
            tracing::info!("a newer snapshot is installed, do not store the built one: {}", meta);
            return Ok(());
        }

        let mut batch = self.db.batch().durability(Some(PersistMode::SyncAll));
        batch.insert(&self.snapshot, SNAPSHOT_META, serialize(meta)?);
        batch.insert(&self.snapshot, SNAPSHOT_DATA, data);
        batch.commit().map_err(io::Error::other)
    }

    /// Replace the whole state with the snapshot `data`, and store it as the current snapshot.
    fn install(&self, meta: &SnapshotMetaOf<C>, data: Vec<u8>) -> Result<(), io::Error> {
        let kvs: BTreeMap<String, types_kv::VersionedValue> = deserialize(&data)?;

        let _guard = self.snapshot_lock.lock().unwrap_or_else(PoisonError::into_inner);

        // One atomic batch: a crash leaves either the old state or the snapshot, never a mix.
        let mut batch = self.db.batch().durability(Some(PersistMode::SyncAll));

        // Remove the keys the snapshot does not have. The keys it has are overwritten below
        // instead, so that a batch never writes a key twice.
        for item in self.data.iter() {
            let key = item.key().map_err(io::Error::other)?;
            let in_snapshot = std::str::from_utf8(&key).is_ok_and(|key| kvs.contains_key(key));
            if !in_snapshot {
                batch.remove(&self.data, key);
            }
        }
        for (key, value) in &kvs {
            batch.insert(&self.data, key.as_str(), serialize(value)?);
        }

        match &meta.last_log_id {
            Some(log_id) => batch.insert(&self.meta, LAST_APPLIED_LOG, serialize(log_id)?),
            None => batch.remove(&self.meta, LAST_APPLIED_LOG),
        }
        batch.insert(&self.meta, LAST_MEMBERSHIP, serialize(&meta.last_membership)?);

        batch.insert(&self.snapshot, SNAPSHOT_META, serialize(meta)?);
        batch.insert(&self.snapshot, SNAPSHOT_DATA, data);

        batch.commit().map_err(io::Error::other)
    }
}

impl<C> RaftStateMachine<C> for FjallStateMachine<C>
where C: RaftTypeConfig<
            D = types_kv::Request,
            R = types_kv::Response,
            Payload = EntryPayloadOf<C>,
            Entry = DefaultEntryOf<C>,
        >
{
    type SnapshotData = Cursor<Vec<u8>>;

    type SnapshotBuilder = FjallSnapshotBuilder<C>;

    async fn applied_state(&mut self) -> Result<(Option<LogIdOf<C>>, StoredMembershipOf<C>), io::Error> {
        let applied = self.read_applied(&self.db.snapshot())?;
        Ok((applied.last_log_id, applied.last_membership))
    }

    async fn apply<Strm>(&mut self, mut entries: Strm) -> Result<(), io::Error>
    where Strm: Stream<Item = Result<EntryResponder<C>, io::Error>> + Unpin + OptionalSend {
        let mut last_applied_log = None;
        let mut last_membership = None;

        // Values set by the entries applied in this call. They reach the database only when the
        // batch is committed, so a compare-and-set must look here first.
        let mut updates = BTreeMap::<String, types_kv::VersionedValue>::new();
        let mut responses = Vec::new();

        while let Some((entry, responder)) = entries.try_next().await? {
            tracing::debug!(%entry.log_id, "replicate to sm");

            let log_id = entry.log_id();
            let version = log_id.index();

            let put = match entry.payload {
                EntryPayload::Blank => None,
                EntryPayload::Normal(types_kv::Request::Set { key, value }) => Some((key, value)),
                EntryPayload::Normal(types_kv::Request::CompareAndSet {
                    key,
                    expected_version,
                    value,
                }) => {
                    let current = match updates.get(&key) {
                        Some(current) => Some(current.clone()),
                        None => self.get(&key)?,
                    };

                    let matches = current.is_some_and(|current| current.version == expected_version);
                    matches.then_some((key, value))
                }
                EntryPayload::Membership(membership) => {
                    last_membership = Some(StoredMembershipOf::<C>::new(Some(log_id.clone()), membership));
                    None
                }
            };

            let response = match put {
                Some((key, value)) => {
                    let response = types_kv::Response::new(value.clone(), version);
                    updates.insert(key, types_kv::VersionedValue { value, version });
                    response
                }
                None => types_kv::Response::none(),
            };

            last_applied_log = Some(log_id);

            if let Some(responder) = responder {
                responses.push((responder, response));
            }
        }

        let Some(last_applied_log) = last_applied_log else {
            return Ok(());
        };

        // Persist data and metadata in one atomic batch, and sync it before acknowledging the
        // applied entries.
        let mut batch = self.db.batch().durability(Some(PersistMode::SyncAll));
        for (key, value) in &updates {
            batch.insert(&self.data, key.as_str(), serialize(value)?);
        }
        batch.insert(&self.meta, LAST_APPLIED_LOG, serialize(&last_applied_log)?);
        if let Some(membership) = &last_membership {
            batch.insert(&self.meta, LAST_MEMBERSHIP, serialize(membership)?);
        }

        C::spawn_blocking(move || batch.commit()).await?.map_err(io::Error::other)?;

        // Only send responses after the write is durable.
        for (responder, response) in responses {
            responder.send(response);
        }

        Ok(())
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        // `&mut self` keeps `apply()` out, so the view holds exactly the applied state.
        FjallSnapshotBuilder::new(self.clone(), self.db.snapshot())
    }

    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMetaOf<C>,
        snapshot: Self::SnapshotData,
    ) -> Result<(), io::Error> {
        let sm = self.clone();
        let meta = meta.clone();
        C::spawn_blocking(move || sm.install(&meta, snapshot.into_inner())).await?
    }

    async fn get_current_snapshot(&mut self) -> Result<Option<SnapshotOf<C, Self::SnapshotData>>, io::Error> {
        // Read both keys from one view: they are always written by the same batch.
        let view = self.db.snapshot();

        let Some(meta) = read::<SnapshotMetaOf<C>, _>(&view, &self.snapshot, SNAPSHOT_META)? else {
            return Ok(None);
        };

        let data = view
            .get(&self.snapshot, SNAPSHOT_DATA)
            .map_err(io::Error::other)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "snapshot data is missing"))?;

        Ok(Some(SnapshotOf::<C, Self::SnapshotData> {
            meta,
            snapshot: Cursor::new(data.to_vec()),
        }))
    }
}

/// Read and decode the value stored under `key` in `keyspace`, as seen by `view`.
fn read<T, R>(view: &R, keyspace: &Keyspace, key: &str) -> Result<Option<T>, io::Error>
where
    T: DeserializeOwned,
    R: Readable,
{
    let bytes = view.get(keyspace, key).map_err(io::Error::other)?;
    bytes.map(|bytes| deserialize(&bytes)).transpose()
}

fn serialize<T>(value: &T) -> Result<Vec<u8>, io::Error>
where T: Serialize {
    serde_json::to_vec(value).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn deserialize<T>(bytes: &[u8]) -> Result<T, io::Error>
where T: DeserializeOwned {
    serde_json::from_slice(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod state_machine_test;
