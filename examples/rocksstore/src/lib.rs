//! This rocks-db backed storage implement the v2 storage API: [`RaftLogStorage`] and
//! [`RaftStateMachine`] traits. Its state machine is pure in-memory store with persisted
//! snapshot. In other words, `applying` a log entry does not flush data to disk at once.
//! These data will be flushed to disk when a snapshot is created.
#![deny(unused_crate_dependencies)]
#![deny(unused_qualifications)]

pub mod log_store;

#[cfg(test)]
mod test;

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use log_store::RocksLogStore;
use openraft::alias::SnapshotDataOf;
use openraft::storage::RaftStateMachine;
use openraft::storage::Snapshot;
use openraft::AnyError;
use openraft::Entry;
use openraft::EntryPayload;
use openraft::LogId;
use openraft::RaftLogId;
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
// #![deny(unused_crate_dependencies)]
// To make the above rule happy, tokio is used, but only in tests
use tokio as _;

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

/**
 * Here you will defined what type of answer you expect from reading the data of a node.
 * In this example it will return a optional value from a given key in
 * the `RocksRequest.Set`.
 */
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RocksResponse {
    pub value: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RocksSnapshot {
    pub meta: SnapshotMeta<TypeConfig>,

    /// The data of the state machine at the time of this snapshot.
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
#[derive(Default)]
#[derive(Serialize, Deserialize)]
pub struct StateMachine {
    pub last_applied_log: Option<LogId<TypeConfig>>,

    pub last_membership: StoredMembership<TypeConfig>,

    /// Application data.
    pub data: BTreeMap<String, String>,
}

// TODO: restore state from snapshot
/// State machine in this implementation is a pure in-memory store.
/// It depends on the latest snapshot to restore the state when restarted.
#[derive(Debug, Clone)]
pub struct RocksStateMachine {
    db: Arc<DB>,
    sm: StateMachine,
}

impl RocksStateMachine {
    async fn new(db: Arc<DB>) -> RocksStateMachine {
        let mut state_machine = Self {
            db,
            sm: Default::default(),
        };
        let snapshot = state_machine.get_current_snapshot().await.unwrap();

        // Restore previous state from snapshot
        if let Some(s) = snapshot {
            let prev: StateMachine = serde_json::from_slice(s.snapshot.get_ref()).unwrap();
            state_machine.sm = prev;
        }

        state_machine
    }
}

impl RaftSnapshotBuilder<TypeConfig> for RocksStateMachine {
    #[tracing::instrument(level = "trace", skip(self))]
    async fn build_snapshot(&mut self) -> Result<Snapshot<TypeConfig>, StorageError<TypeConfig>> {
        // Serialize the data of the state machine.
        let data = serde_json::to_vec(&self.sm).map_err(|e| StorageError::read_state_machine(&e))?;

        let last_applied_log = self.sm.last_applied_log;
        let last_membership = self.sm.last_membership.clone();

        // Generate a random snapshot index.
        let snapshot_idx: u64 = rand::thread_rng().gen_range(0..1000);

        let snapshot_id = if let Some(last) = last_applied_log {
            format!("{}-{}-{}", last.leader_id, last.index, snapshot_idx)
        } else {
            format!("--{}", snapshot_idx)
        };

        let meta = SnapshotMeta {
            last_log_id: last_applied_log,
            last_membership,
            snapshot_id,
        };

        let snapshot = RocksSnapshot {
            meta: meta.clone(),
            data: data.clone(),
        };

        let serialized_snapshot = serde_json::to_vec(&snapshot)
            .map_err(|e| StorageError::write_snapshot(Some(meta.signature()), AnyError::new(&e)))?;

        self.db
            .put_cf(self.db.cf_handle("sm_meta").unwrap(), "snapshot", serialized_snapshot)
            .map_err(|e| StorageError::write_snapshot(Some(meta.signature()), AnyError::new(&e)))?;

        Ok(Snapshot {
            meta,
            snapshot: Box::new(Cursor::new(data)),
        })
    }
}

impl RaftStateMachine<TypeConfig> for RocksStateMachine {
    type SnapshotBuilder = Self;

    async fn applied_state(
        &mut self,
    ) -> Result<(Option<LogId<TypeConfig>>, StoredMembership<TypeConfig>), StorageError<TypeConfig>> {
        Ok((self.sm.last_applied_log, self.sm.last_membership.clone()))
    }

    async fn apply<I>(&mut self, entries: I) -> Result<Vec<RocksResponse>, StorageError<TypeConfig>>
    where I: IntoIterator<Item = Entry<TypeConfig>> + Send {
        let entries_iter = entries.into_iter();
        let mut res = Vec::with_capacity(entries_iter.size_hint().0);

        let sm = &mut self.sm;

        for entry in entries_iter {
            tracing::debug!(%entry.log_id, "replicate to sm");

            sm.last_applied_log = Some(*entry.get_log_id());

            match entry.payload {
                EntryPayload::Blank => res.push(RocksResponse { value: None }),
                EntryPayload::Normal(ref req) => match req {
                    RocksRequest::Set { key, value } => {
                        sm.data.insert(key.clone(), value.clone());
                        res.push(RocksResponse {
                            value: Some(value.clone()),
                        })
                    }
                },
                EntryPayload::Membership(ref mem) => {
                    sm.last_membership = StoredMembership::new(Some(entry.log_id), mem.clone());
                    res.push(RocksResponse { value: None })
                }
            };
        }
        Ok(res)
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        self.clone()
    }

    async fn begin_receiving_snapshot(&mut self) -> Result<Box<SnapshotDataOf<TypeConfig>>, StorageError<TypeConfig>> {
        Ok(Box::new(Cursor::new(Vec::new())))
    }

    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta<TypeConfig>,
        snapshot: Box<SnapshotDataOf<TypeConfig>>,
    ) -> Result<(), StorageError<TypeConfig>> {
        tracing::info!(
            { snapshot_size = snapshot.get_ref().len() },
            "decoding snapshot for installation"
        );

        let new_snapshot = RocksSnapshot {
            meta: meta.clone(),
            data: snapshot.into_inner(),
        };

        // Update the state machine.
        let updated_state_machine: StateMachine = serde_json::from_slice(&new_snapshot.data)
            .map_err(|e| StorageError::read_snapshot(Some(new_snapshot.meta.signature()), &e))?;

        self.sm = updated_state_machine;

        // Save snapshot

        let serialized_snapshot = serde_json::to_vec(&new_snapshot)
            .map_err(|e| StorageError::write_snapshot(Some(meta.signature()), AnyError::new(&e)))?;

        self.db
            .put_cf(self.db.cf_handle("sm_meta").unwrap(), "snapshot", serialized_snapshot)
            .map_err(|e| StorageError::write_snapshot(Some(meta.signature()), AnyError::new(&e)))?;

        self.db.flush_wal(true).map_err(|e| StorageError::write_snapshot(Some(meta.signature()), &e))?;
        Ok(())
    }

    async fn get_current_snapshot(&mut self) -> Result<Option<Snapshot<TypeConfig>>, StorageError<TypeConfig>> {
        let x = self
            .db
            .get_cf(self.db.cf_handle("sm_meta").unwrap(), "snapshot")
            .map_err(|e| StorageError::write_snapshot(None, AnyError::new(&e)))?;

        let bytes = match x {
            Some(x) => x,
            None => return Ok(None),
        };

        let snapshot: RocksSnapshot =
            serde_json::from_slice(&bytes).map_err(|e| StorageError::write_snapshot(None, AnyError::new(&e)))?;

        let data = snapshot.data.clone();

        Ok(Some(Snapshot {
            meta: snapshot.meta,
            snapshot: Box::new(Cursor::new(data)),
        }))
    }
}

/// Create a pair of `RocksLogStore` and `RocksStateMachine` that are backed by a same rocks db
/// instance.
pub async fn new<C, P: AsRef<Path>>(db_path: P) -> (RocksLogStore<C>, RocksStateMachine)
where C: RaftTypeConfig {
    let mut db_opts = Options::default();
    db_opts.create_missing_column_families(true);
    db_opts.create_if_missing(true);

    let meta = ColumnFamilyDescriptor::new("meta", Options::default());
    let sm_meta = ColumnFamilyDescriptor::new("sm_meta", Options::default());
    let logs = ColumnFamilyDescriptor::new("logs", Options::default());

    let db = DB::open_cf_descriptors(&db_opts, db_path, vec![meta, sm_meta, logs]).unwrap();

    let db = Arc::new(db);
    (RocksLogStore::new(db.clone()), RocksStateMachine::new(db).await)
}
