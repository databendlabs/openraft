#![doc = include_str!("../README.md")]
#![feature(backtrace)]

mod config;
mod core;
mod defensive;
mod membership;
mod raft_types;
mod replication;
mod storage_error;
mod store_ext;
mod store_wrapper;
mod summary;

pub mod error;
pub mod metrics;
pub mod network;
pub mod raft;
pub mod storage;
pub mod testing;
pub mod types;

#[cfg(test)]
mod metrics_wait_test;

pub use async_trait;

pub use crate::config::Config;
pub use crate::config::ConfigError;
pub use crate::config::SnapshotPolicy;
pub use crate::core::State;
pub use crate::defensive::DefensiveCheck;
pub use crate::metrics::RaftMetrics;
pub use crate::raft::Raft;
pub use crate::raft_types::LogIdOptionExt;
pub use crate::raft_types::Update;
pub use crate::replication::ReplicationMetrics;
pub use crate::store_ext::StoreExt;
pub use crate::store_wrapper::Wrapper;
pub use crate::summary::MessageSummary;
pub use crate::types::v070::AppData;
pub use crate::types::v070::AppDataResponse;
pub use crate::types::v070::AppendEntriesRequest;
pub use crate::types::v070::AppendEntriesResponse;
pub use crate::types::v070::DefensiveError;
pub use crate::types::v070::EffectiveMembership;
pub use crate::types::v070::Entry;
pub use crate::types::v070::EntryPayload;
pub use crate::types::v070::ErrorSubject;
pub use crate::types::v070::ErrorVerb;
pub use crate::types::v070::HardState;
pub use crate::types::v070::InitialState;
pub use crate::types::v070::LogId;
pub use crate::types::v070::LogState;
pub use crate::types::v070::Membership;
pub use crate::types::v070::NodeId;
pub use crate::types::v070::RaftNetwork;
pub use crate::types::v070::RaftStorage;
pub use crate::types::v070::RaftStorageDebug;
pub use crate::types::v070::Snapshot;
pub use crate::types::v070::SnapshotId;
pub use crate::types::v070::SnapshotMeta;
pub use crate::types::v070::SnapshotSegmentId;
pub use crate::types::v070::StateMachineChanges;
pub use crate::types::v070::StorageError;
pub use crate::types::v070::StorageIOError;
pub use crate::types::v070::Violation;
