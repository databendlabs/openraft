//! This rocks-db backed storage implement the v2 storage API: [`RaftLogStorage`] and
//! [`RaftStateMachine`] traits. The state machine stores all data directly in RocksDB,
//! providing full persistence. Log entries are applied directly to disk, and snapshots
//! use RocksDB's snapshot mechanism for consistent point-in-time views.
#![deny(unused_crate_dependencies)]
#![deny(unused_qualifications)]
#![allow(clippy::uninlined_format_args)]

pub mod log_store;

#[cfg(test)]
mod test;
use std::fmt;
use std::fmt::Debug;
use std::fs;
use std::io;
use std::io::Cursor;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use futures::Stream;
use futures::TryStreamExt;
use openraft::alias::SnapshotDataOf;
use openraft::entry::RaftEntry;
use openraft::storage::EntryResponder;
use openraft::storage::RaftStateMachine;
use openraft::storage::Snapshot;
use openraft::AnyError;
use openraft::EntryPayload;
use openraft::LogId;
use openraft::OptionalSend;
use openraft::RaftSnapshotBuilder;
use openraft::RaftTypeConfig;
use openraft::SnapshotMeta;
use openraft::StorageError;
use openraft::StoredMembership;
use rand::Rng;
use rocksdb::ColumnFamilyDescriptor;
use rocksdb::Options;
use rocksdb::DB;
use serde::Deserialize;
use serde::Serialize;
use tokio::task::spawn_blocking;

use crate::log_store::RocksLogStore;

pub type RocksNodeId = u64;

openraft::declare_raft_types!(
    /// Declare the type configuration.
    pub TypeConfig:
        D = RocksRequest,
        R = RocksResponse,
);

/**
 * Here you will set the types of request that will interact with the raft nodes.
 * For example the `Set` will be used to write data (key and value) to the raft database.
 * The `AddNode` will append a new node to the current existing shared list of nodes.
 * You will want to add any request that can write data in all nodes here.
 */
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum RocksRequest {
    Set { key: String, value: String },
}

impl fmt::Display for RocksRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RocksRequest::Set { key, value } => write!(f, "Set {{ key: {}, value: {} }}", key, value),
        }
    }
}

/**
 * Here you will define what type of answer you expect from reading the data of a node.
 * In this example it will return a optional value from a given key in
 * the `RocksRequest.Set`.
 */
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RocksResponse {
    pub value: Option<String>,
}

/// State machine backed by RocksDB for full persistence.
/// All application data is stored directly in the `sm_data` column family.
/// Snapshots are persisted to the `snapshot_dir` directory.
#[derive(Debug, Clone)]
pub struct RocksStateMachine {
    db: Arc<DB>,
    snapshot_dir: PathBuf,
}

impl RocksStateMachine {
    async fn new(db: Arc<DB>, snapshot_dir: PathBuf) -> Result<RocksStateMachine, io::Error> {
        // Validate column families exist at construction time
        db.cf_handle("sm_meta").ok_or_else(|| io::Error::other("column family `sm_meta` not found"))?;
        db.cf_handle("sm_data").ok_or_else(|| io::Error::other("column family `sm_data` not found"))?;

        // Create snapshot directory if it doesn't exist
        fs::create_dir_all(&snapshot_dir)?;

        Ok(Self { db, snapshot_dir })
    }

    fn cf_sm_meta(&self) -> &rocksdb::ColumnFamily {
        self.db.cf_handle("sm_meta").unwrap()
    }

    fn cf_sm_data(&self) -> &rocksdb::ColumnFamily {
        self.db.cf_handle("sm_data").unwrap()
    }

    #[allow(clippy::type_complexity)]
    fn get_meta(&self) -> Result<(Option<LogId<TypeConfig>>, StoredMembership<TypeConfig>), StorageError<TypeConfig>> {
        let cf = self.cf_sm_meta();

        let last_applied_log = self
            .db
            .get_cf(cf, "last_applied_log")
            .map_err(|e| StorageError::read(&e))?
            .map(|bytes| deserialize(&bytes))
            .transpose()?;

        let last_membership = self
            .db
            .get_cf(cf, "last_membership")
            .map_err(|e| StorageError::read(&e))?
            .map(|bytes| deserialize(&bytes))
            .transpose()?
            .unwrap_or_default();

        Ok((last_applied_log, last_membership))
    }
}

fn serialize<T: Serialize>(value: &T) -> Result<Vec<u8>, StorageError<TypeConfig>> {
    serde_json::to_vec(value).map_err(|e| StorageError::write(&e))
}

fn deserialize<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, StorageError<TypeConfig>> {
    serde_json::from_slice(bytes).map_err(|e| StorageError::read(&e))
}

/// Snapshot file format: metadata + data stored together
#[derive(Serialize, Deserialize)]
struct SnapshotFile {
    meta: SnapshotMeta<TypeConfig>,
    data: Vec<(Vec<u8>, Vec<u8>)>,
}

impl RaftSnapshotBuilder<TypeConfig> for RocksStateMachine {
    #[tracing::instrument(level = "trace", skip(self))]
    async fn build_snapshot(&mut self) -> Result<Snapshot<TypeConfig>, io::Error> {
        let (last_applied_log, last_membership) = self.get_meta()?;

        // Generate a random snapshot index.
        let snapshot_idx: u64 = rand::rng().random_range(0..1000);

        let snapshot_id = if let Some(last) = last_applied_log {
            format!("{}-{}-{}", last.committed_leader_id(), last.index(), snapshot_idx)
        } else {
            format!("--{}", snapshot_idx)
        };

        let meta = SnapshotMeta {
            last_log_id: last_applied_log,
            last_membership,
            snapshot_id: snapshot_id.clone(),
        };

        // Use RocksDB snapshot for consistent point-in-time view
        let db = self.db.clone();

        #[allow(clippy::type_complexity)]
        let data = spawn_blocking(move || -> Result<Vec<(Vec<u8>, Vec<u8>)>, io::Error> {
            let snapshot = db.snapshot();
            let cf_data = db.cf_handle("sm_data").expect("column family `sm_data` not found");

            let mut snapshot_data = Vec::new();
            let iter = snapshot.iterator_cf(cf_data, rocksdb::IteratorMode::Start);

            for item in iter {
                let (key, value) = item.map_err(|e| io::Error::other(e.to_string()))?;
                snapshot_data.push((key.to_vec(), value.to_vec()));
            }

            Ok(snapshot_data)
        })
        .await
        .map_err(|e| io::Error::other(e.to_string()))??;

        // Serialize both metadata and data together
        let snapshot_file = SnapshotFile {
            meta: meta.clone(),
            data: data.clone(),
        };
        let file_bytes = serialize(&snapshot_file)
            .map_err(|e| StorageError::write_snapshot(Some(meta.signature()), AnyError::new(&e)))?;

        // Write complete snapshot to file
        let snapshot_path = self.snapshot_dir.join(&snapshot_id);
        fs::write(&snapshot_path, &file_bytes).map_err(|e| StorageError::write_snapshot(Some(meta.signature()), &e))?;

        // Return snapshot with data-only for backward compatibility with the data field
        let data_bytes =
            serialize(&data).map_err(|e| StorageError::write_snapshot(Some(meta.signature()), AnyError::new(&e)))?;

        Ok(Snapshot {
            meta,
            snapshot: Cursor::new(data_bytes),
        })
    }
}

impl RaftStateMachine<TypeConfig> for RocksStateMachine {
    type SnapshotBuilder = Self;

    async fn applied_state(&mut self) -> Result<(Option<LogId<TypeConfig>>, StoredMembership<TypeConfig>), io::Error> {
        self.get_meta().map_err(|e| io::Error::other(e.to_string()))
    }

    async fn apply<Strm>(&mut self, mut entries: Strm) -> Result<(), io::Error>
    where Strm: Stream<Item = Result<EntryResponder<TypeConfig>, io::Error>> + Unpin + OptionalSend {
        let mut batch = rocksdb::WriteBatch::default();
        let mut last_applied_log = None;
        let mut last_membership = None;
        let mut responses = Vec::new();

        while let Some((entry, responder)) = entries.try_next().await? {
            tracing::debug!(%entry.log_id, "replicate to sm");

            last_applied_log = Some(entry.log_id());

            let response = match entry.payload {
                EntryPayload::Blank => RocksResponse { value: None },
                EntryPayload::Normal(ref req) => match req {
                    RocksRequest::Set { key, value } => {
                        let cf_data = self.cf_sm_data();

                        batch.put_cf(cf_data, key.as_bytes(), value.as_bytes());
                        RocksResponse {
                            value: Some(value.clone()),
                        }
                    }
                },
                EntryPayload::Membership(ref mem) => {
                    last_membership = Some(StoredMembership::new(Some(entry.log_id), mem.clone()));
                    RocksResponse { value: None }
                }
            };

            if let Some(responder) = responder {
                responses.push((responder, response));
            }
        }

        let cf_meta = self.cf_sm_meta();

        // Add metadata writes to the batch for atomic commit
        if let Some(ref log_id) = last_applied_log {
            batch.put_cf(cf_meta, "last_applied_log", serialize(log_id)?);
        }

        if let Some(ref membership) = last_membership {
            batch.put_cf(cf_meta, "last_membership", serialize(membership)?);
        }

        // Atomic write of all data + metadata - fail fast before sending any responses
        self.db.write(batch).map_err(|e| io::Error::other(e.to_string()))?;

        // Only send responses after successful write
        for (responder, response) in responses {
            responder.send(response);
        }

        Ok(())
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        self.clone()
    }

    async fn begin_receiving_snapshot(&mut self) -> Result<SnapshotDataOf<TypeConfig>, io::Error> {
        Ok(Cursor::new(Vec::new()))
    }

    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta<TypeConfig>,
        snapshot: SnapshotDataOf<TypeConfig>,
    ) -> Result<(), io::Error> {
        tracing::info!(
            { snapshot_size = snapshot.get_ref().len() },
            "decoding snapshot for installation"
        );

        // Deserialize snapshot data
        let snapshot_data: Vec<(Vec<u8>, Vec<u8>)> =
            deserialize(snapshot.get_ref()).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

        // Clone data for file writing later
        let snapshot_data_clone = snapshot_data.clone();

        // Prepare metadata to restore
        let last_applied_bytes = meta
            .last_log_id
            .as_ref()
            .map(|log_id| serialize(log_id).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string())))
            .transpose()?;

        let last_membership_bytes =
            serialize(&meta.last_membership).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

        // Restore data and metadata atomically to RocksDB
        let db = self.db.clone();

        spawn_blocking(move || -> Result<(), io::Error> {
            let cf_data = db.cf_handle("sm_data").expect("column family `sm_data` not found");
            let cf_meta = db.cf_handle("sm_meta").expect("column family `sm_meta` not found");

            let mut batch = rocksdb::WriteBatch::default();

            // Clear existing data in sm_data
            let iter = db.iterator_cf(cf_data, rocksdb::IteratorMode::Start);
            for item in iter {
                let (key, _) = item.map_err(|e| io::Error::other(e.to_string()))?;
                batch.delete_cf(cf_data, &key);
            }

            // Restore snapshot data to sm_data
            for (key, value) in snapshot_data {
                batch.put_cf(cf_data, &key, &value);
            }

            // Restore metadata to sm_meta
            if let Some(bytes) = last_applied_bytes {
                batch.put_cf(cf_meta, "last_applied_log", bytes);
            }
            batch.put_cf(cf_meta, "last_membership", last_membership_bytes);

            // Atomic write of all changes
            db.write(batch).map_err(|e| io::Error::other(e.to_string()))?;

            db.flush_wal(true).map_err(|e| io::Error::other(e.to_string()))
        })
        .await
        .map_err(|e| io::Error::other(e.to_string()))??;

        // Write snapshot file with metadata for get_current_snapshot
        let snapshot_file = SnapshotFile {
            meta: meta.clone(),
            data: snapshot_data_clone,
        };
        let file_bytes =
            serialize(&snapshot_file).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

        let snapshot_path = self.snapshot_dir.join(&meta.snapshot_id);
        fs::write(&snapshot_path, &file_bytes)?;

        Ok(())
    }

    async fn get_current_snapshot(&mut self) -> Result<Option<Snapshot<TypeConfig>>, io::Error> {
        // Find the latest snapshot file by comparing filenames lexicographically
        let mut latest_snapshot_id: Option<String> = None;

        for entry in fs::read_dir(&self.snapshot_dir)? {
            let entry = entry?;
            let path = entry.path();

            if !path.is_file() {
                continue;
            }

            if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                let snapshot_id = filename.to_string();

                // Update latest if this is the first snapshot or if it's newer
                if latest_snapshot_id.as_ref().is_none_or(|current| snapshot_id > *current) {
                    latest_snapshot_id = Some(snapshot_id);
                }
            }
        }

        let Some(snapshot_id) = latest_snapshot_id else {
            return Ok(None);
        };

        let snapshot_path = self.snapshot_dir.join(&snapshot_id);

        // Read and deserialize snapshot file
        let file_bytes = fs::read(&snapshot_path)?;
        let snapshot_file: SnapshotFile =
            deserialize(&file_bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

        // Serialize data for snapshot field
        let data_bytes =
            serialize(&snapshot_file.data).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

        Ok(Some(Snapshot {
            meta: snapshot_file.meta,
            snapshot: Cursor::new(data_bytes),
        }))
    }
}

/// Create a pair of `RocksLogStore` and `RocksStateMachine` that are backed by a same rocks db
/// instance.
pub async fn new<C, P: AsRef<Path>>(db_path: P) -> Result<(RocksLogStore<C>, RocksStateMachine), io::Error>
where C: RaftTypeConfig {
    let mut db_opts = Options::default();
    db_opts.create_missing_column_families(true);
    db_opts.create_if_missing(true);

    let meta = ColumnFamilyDescriptor::new("meta", Options::default());
    let sm_meta = ColumnFamilyDescriptor::new("sm_meta", Options::default());
    let sm_data = ColumnFamilyDescriptor::new("sm_data", Options::default());
    let logs = ColumnFamilyDescriptor::new("logs", Options::default());

    let db_path = db_path.as_ref();
    let snapshot_dir = db_path.join("snapshots");

    let db =
        DB::open_cf_descriptors(&db_opts, db_path, vec![meta, sm_meta, sm_data, logs]).map_err(io::Error::other)?;

    let db = Arc::new(db);
    Ok((
        RocksLogStore::new(db.clone()),
        RocksStateMachine::new(db, snapshot_dir).await?,
    ))
}
