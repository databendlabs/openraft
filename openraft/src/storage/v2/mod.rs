//! Defines [`RaftLogStorage`] and [`RaftStateMachine`] trait.
//!
//! [`RaftLogStorage`] is responsible for storing logs,
//! and [`RaftStateMachine`] is responsible for storing state machine and snapshot.

mod raft_log_reader;
mod raft_log_storage;
mod raft_log_storage_ext;
mod raft_snapshot_builder;
mod raft_state_machine;

pub use self::raft_log_reader::RaftLogReader;
pub use self::raft_log_storage::RaftLogStorage;
pub use self::raft_log_storage_ext::RaftLogStorageExt;
pub use self::raft_snapshot_builder::RaftSnapshotBuilder;
pub use self::raft_state_machine::RaftStateMachine;
