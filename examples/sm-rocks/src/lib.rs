//! This RocksDB-backed storage implements the [`openraft::storage::RaftStateMachine`] trait.
//! The state machine stores all data directly in RocksDB,
//! providing full persistence. Entries are applied directly to disk, and snapshots
//! are RocksDB checkpoints: hard-linked copies of the database that share its SST files.
#![deny(unused_crate_dependencies)]
#![deny(unused_qualifications)]
#![allow(clippy::uninlined_format_args)]

mod snapshot;
pub mod state_machine;

#[cfg(test)]
mod test;

pub use crate::snapshot::RocksSnapshotBuilder;
pub use crate::snapshot::RocksSnapshotData;
pub use crate::state_machine::RocksStateMachine;

#[cfg(test)]
openraft::declare_raft_types!(
    /// The type configuration the tests run with.
    pub TypeConfig:
        D = types_kv::Request,
        R = types_kv::Response,
);
