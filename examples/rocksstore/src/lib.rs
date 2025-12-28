//! This rocks-db backed storage implement the v2 storage API: [`RaftLogStorage`] and
//! [`RaftStateMachine`] traits. The state machine stores all data directly in RocksDB,
//! providing full persistence. Log entries are applied directly to disk, and snapshots
//! use RocksDB's snapshot mechanism for consistent point-in-time views.
#![deny(unused_crate_dependencies)]
#![deny(unused_qualifications)]
#![allow(clippy::uninlined_format_args)]

pub mod log_store;
pub mod state_machine;

#[cfg(test)]
mod test;

use std::io;
use std::path::Path;
use std::sync::Arc;

use openraft::RaftTypeConfig;
use rocksdb::ColumnFamilyDescriptor;
use rocksdb::DB;
use rocksdb::Options;

use crate::log_store::RocksLogStore;
pub use crate::state_machine::RocksStateMachine;

pub type RocksNodeId = u64;

openraft::declare_raft_types!(
    /// Declare the type configuration.
    pub TypeConfig:
        D = types_kv::Request,
        R = types_kv::Response,
);

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
