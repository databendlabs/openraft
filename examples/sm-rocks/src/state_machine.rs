//! RocksDB-backed state machine implementation.

pub(crate) mod active_db;

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

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
use rocksdb::DB;
use rocksdb::WriteOptions;
use serde::Deserialize;
use serde::Serialize;

use self::active_db::ActiveDb;
use crate::RocksSnapshotBuilder;
use crate::RocksSnapshotData;

/// State machine backed by RocksDB for full persistence.
///
/// Application data lives in the `sm_data` column family and metadata in `sm_meta`.
/// Every database is a generation directory under `base_dir`: the active writable database,
/// checkpoints captured for snapshots, and checkpoints received from a leader. The `CURRENT`
/// file names the active generation. Installing a snapshot switches `CURRENT` to a writable
/// copy of the received checkpoint instead of rewriting keys.
#[derive(Debug)]
pub struct RocksStateMachine<C>
where C: RaftTypeConfig
{
    /// Directory holding `CURRENT` and every generation.
    base_dir: PathBuf,

    /// The active writable generation, shared with the blocking tasks that write and
    /// checkpoint it.
    active: Arc<ActiveDb>,

    /// The newest snapshot built or installed since startup.
    current_snapshot: Option<SnapshotOf<C, RocksSnapshotData>>,
}

impl<C> RocksStateMachine<C>
where C: RaftTypeConfig
{
    /// Open the state machine stored under `base_dir`, creating it when absent.
    ///
    /// The log store lives in the `log-rocks` crate and is opened separately.
    pub fn open<P>(base_dir: P) -> Result<Self, io::Error>
    where P: AsRef<Path> {
        let base_dir = base_dir.as_ref().to_path_buf();
        let active = ActiveDb::open_or_create(&base_dir)?;

        Ok(Self {
            base_dir,
            active,
            current_snapshot: None,
        })
    }

    fn get_meta(&self) -> io::Result<SnapshotMetaOf<C>> {
        read_meta::<C>(self.active.db())
    }

    /// Capture a checkpoint of the active database on a blocking thread.
    async fn checkpoint_active_db(&self) -> io::Result<SnapshotOf<C, RocksSnapshotData>> {
        let active = Arc::clone(&self.active);
        let base_dir = self.base_dir.clone();
        C::spawn_blocking(move || checkpoint_snapshot::<C>(active.db(), &base_dir)).await?
    }
}

impl<C> RaftStateMachine<C> for RocksStateMachine<C>
where C: RaftTypeConfig<
            D = types_kv::Request,
            R = types_kv::Response,
            Payload = EntryPayloadOf<C>,
            Entry = DefaultEntryOf<C>,
        >
{
    type SnapshotData = RocksSnapshotData;

    type SnapshotBuilder = RocksSnapshotBuilder<C>;

    async fn applied_state(&mut self) -> Result<(Option<LogIdOf<C>>, StoredMembershipOf<C>), io::Error> {
        let meta = self.get_meta()?;
        Ok((meta.last_log_id, meta.last_membership))
    }

    async fn apply<Strm>(&mut self, mut entries: Strm) -> Result<(), io::Error>
    where Strm: Stream<Item = Result<EntryResponder<C>, io::Error>> + Unpin + OptionalSend {
        let db = self.active.db();
        let mut batch = rocksdb::WriteBatch::default();
        let mut last_applied_log = None;
        let mut last_membership: Option<StoredMembershipOf<C>> = None;
        let mut pending_values = BTreeMap::<String, types_kv::VersionedValue>::new();
        let mut responses = Vec::new();

        while let Some((entry, responder)) = entries.try_next().await? {
            tracing::debug!(%entry.log_id, "replicate to sm");

            // Look the handle up per entry: `ColumnFamily` is not `Sync`, so holding it across
            // the `await` in the loop head would make this future `!Send`.
            let cf_data = column_family(db, "sm_data")?;
            let version = entry.log_id().index();
            last_applied_log = Some(entry.log_id());

            let put = match entry.payload {
                EntryPayload::Blank => None,
                EntryPayload::Normal(ref req) => match req {
                    types_kv::Request::Set { key, value } => Some((key, value)),
                    types_kv::Request::CompareAndSet {
                        key,
                        expected_version,
                        value,
                    } => {
                        let current = if let Some(current) = pending_values.get(key) {
                            Some(current.clone())
                        } else {
                            db.get_cf(cf_data, key.as_bytes())
                                .map_err(io::Error::other)?
                                .map(|bytes| deserialize(&bytes))
                                .transpose()?
                        };

                        if current.is_some_and(|current| current.version == *expected_version) {
                            Some((key, value))
                        } else {
                            None
                        }
                    }
                },
                EntryPayload::Membership(ref mem) => {
                    last_membership = Some(StoredMembershipOf::<C>::new(Some(entry.log_id), mem.clone()));
                    None
                }
            };

            let response = match put {
                Some((key, value)) => {
                    let versioned_value = types_kv::VersionedValue {
                        value: value.clone(),
                        version,
                    };

                    batch.put_cf(cf_data, key.as_bytes(), serialize(&versioned_value)?);
                    pending_values.insert(key.clone(), versioned_value);
                    types_kv::Response::new(value.clone(), version)
                }
                None => types_kv::Response::none(),
            };

            if let Some(responder) = responder {
                responses.push((responder, response));
            }
        }

        let cf_meta = column_family(db, "sm_meta")?;

        // Add metadata writes to the batch for atomic commit
        if let Some(ref log_id) = last_applied_log {
            batch.put_cf(cf_meta, "last_applied_log", serialize(log_id)?);
        }

        if let Some(ref membership) = last_membership {
            batch.put_cf(cf_meta, "last_membership", serialize(membership)?);
        }

        // Persist data and metadata atomically before acknowledging the applied entries.
        let active = Arc::clone(&self.active);
        C::spawn_blocking(move || {
            let mut write_options = WriteOptions::default();
            write_options.set_sync(true);
            active.db().write_opt(batch, &write_options).map_err(io::Error::other)
        })
        .await??;

        // Only send responses after the write is durable.
        for (responder, response) in responses {
            responder.send(response);
        }

        Ok(())
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        let prepared = self.checkpoint_active_db().await;
        if let Ok(snapshot) = &prepared {
            self.current_snapshot = Some(snapshot.clone());
        }
        RocksSnapshotBuilder::new(prepared)
    }

    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMetaOf<C>,
        snapshot: Self::SnapshotData,
    ) -> Result<(), io::Error> {
        let base_dir = self.base_dir.clone();
        let received = snapshot.clone();
        let candidate = C::spawn_blocking(move || ActiveDb::from_snapshot(&base_dir, &received)).await??;

        candidate.make_current(&self.base_dir)?;
        self.active.mark_for_removal();
        self.active = candidate;

        self.current_snapshot = Some(SnapshotOf::<C, RocksSnapshotData> {
            meta: meta.clone(),
            snapshot,
        });
        Ok(())
    }

    async fn get_current_snapshot(&mut self) -> Result<Option<SnapshotOf<C, Self::SnapshotData>>, io::Error> {
        Ok(self.current_snapshot.clone())
    }
}

/// Capture a checkpoint of `db` and describe it with the metadata `db` holds.
///
/// The caller holds `&mut` on the state machine, so nothing writes between the two steps.
fn checkpoint_snapshot<C>(db: &DB, base_dir: &Path) -> io::Result<SnapshotOf<C, RocksSnapshotData>>
where C: RaftTypeConfig {
    let meta = read_meta::<C>(db)?;
    let snapshot = RocksSnapshotData::checkpoint(db, base_dir)?;

    Ok(SnapshotOf::<C, RocksSnapshotData> { meta, snapshot })
}

fn column_family<'a>(db: &'a DB, name: &str) -> io::Result<&'a rocksdb::ColumnFamily> {
    db.cf_handle(name).ok_or_else(|| io::Error::other(format!("column family `{name}` not found")))
}

fn read_meta<C>(db: &DB) -> io::Result<SnapshotMetaOf<C>>
where C: RaftTypeConfig {
    let cf = column_family(db, "sm_meta")?;
    let last_log_id = db
        .get_cf(cf, "last_applied_log")
        .map_err(io::Error::other)?
        .map(|bytes| deserialize(&bytes))
        .transpose()?;
    let last_membership = db
        .get_cf(cf, "last_membership")
        .map_err(io::Error::other)?
        .map(|bytes| deserialize(&bytes))
        .transpose()?
        .unwrap_or_default();
    Ok(SnapshotMetaOf::<C> {
        last_log_id,
        last_membership,
    })
}

fn serialize<T>(value: &T) -> io::Result<Vec<u8>>
where T: Serialize {
    serde_json::to_vec(value).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn deserialize<T>(bytes: &[u8]) -> io::Result<T>
where T: for<'de> Deserialize<'de> {
    serde_json::from_slice(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[cfg(test)]
mod queue_transport;
#[cfg(test)]
mod state_machine_test;
