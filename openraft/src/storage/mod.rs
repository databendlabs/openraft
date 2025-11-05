//! The Raft storage interface and data types.
//!
//! This module defines traits and types for implementing Raft log storage and state machine:
//!
//! ## Core Traits
//!
//! - [`RaftLogStorage`] - Persistent log storage for Raft entries and vote state
//! - [`RaftStateMachine`] - Application state machine that applies committed log entries
//! - [`RaftLogReader`] - Reader interface for accessing stored log entries
//! - [`RaftSnapshotBuilder`] - Builder interface for creating snapshots
//!
//! ## Key Types
//!
//! - [`LogState`] - Current state of log storage (first/last log IDs)
//! - [`Snapshot`] - Container for snapshot data and metadata
//! - [`SnapshotMeta`] - Snapshot metadata (last log ID, membership)
//!
//! ## Usage
//!
//! Applications implement [`RaftLogStorage`] and [`RaftStateMachine`] to provide
//! persistence for Raft. These implementations are passed to [`Raft::new()`](crate::Raft::new)
//! to create a Raft node.
//!
//! See the [Getting Started Guide](crate::docs::getting_started) and
//! [State Machine Component](crate::docs::components::state_machine) documentation
//! for implementation details and examples.

mod callback;
mod helper;
mod log_reader_ext;
mod log_state;
mod snapshot;
mod snapshot_meta;
mod snapshot_signature;
pub(crate) mod v2;

pub use self::callback::IOFlushed;
pub use self::callback::LogApplied;
#[allow(deprecated)]
pub use self::callback::LogFlushed;
pub use self::helper::StorageHelper;
pub use self::log_reader_ext::RaftLogReaderExt;
pub use self::log_state::LogState;
pub use self::snapshot::Snapshot;
pub use self::snapshot_meta::SnapshotMeta;
pub use self::snapshot_signature::SnapshotSignature;
pub use self::v2::ApplyResponder;
pub use self::v2::EntryResponder;
pub use self::v2::LeaderBoundedStreamError;
pub use self::v2::LeaderBoundedStreamResult;
pub use self::v2::RaftLogReader;
pub use self::v2::RaftLogStorage;
pub use self::v2::RaftLogStorageExt;
pub use self::v2::RaftSnapshotBuilder;
pub use self::v2::RaftStateMachine;
