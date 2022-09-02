#![doc = include_str!("../README.md")]
#![cfg_attr(feature = "bt", feature(error_generic_member_access))]
#![cfg_attr(feature = "bt", feature(provide_any))]

//! # Feature flags
//!
//! - `bt`: Enable backtrace: generate backtrace for errors. This requires a unstable feature
//!   `error_generic_member_access` and `provide_any` thus it can not be used with stable rust.

pub mod config;
mod core;
pub mod error;
pub mod metrics;
#[cfg(test)]
mod metrics_wait_test;
pub mod network;
mod quorum;
pub mod raft;
mod raft_types;
mod replication;
pub mod storage;
mod storage_error;
mod summary;

mod defensive;
#[cfg(test)]
mod membership_test;
mod store_ext;
mod store_wrapper;

pub mod types;

pub use anyerror;
pub use anyerror::AnyError;
pub use async_trait;
pub use store_ext::StoreExt;
pub use store_wrapper::Wrapper;

pub use crate::config::Config;
pub use crate::config::SnapshotPolicy;
pub use crate::core::State;
pub use crate::defensive::DefensiveCheck;
pub use crate::error::ChangeMembershipError;
pub use crate::error::ClientWriteError;
pub use crate::error::ConfigError;
pub use crate::error::InitializeError;
pub use crate::error::RaftError;
pub use crate::error::ReplicationError;
pub use crate::metrics::RaftMetrics;
pub use crate::raft::Raft;
pub use crate::raft_types::Update;
pub use crate::replication::ReplicationMetrics;
pub use crate::summary::MessageSummary;
pub use crate::types::v065::AppData;
pub use crate::types::v065::AppDataResponse;
pub use crate::types::v065::AppendEntriesRequest;
pub use crate::types::v065::AppendEntriesResponse;
pub use crate::types::v065::DefensiveError;
pub use crate::types::v065::EffectiveMembership;
pub use crate::types::v065::Entry;
pub use crate::types::v065::EntryPayload;
pub use crate::types::v065::ErrorSubject;
pub use crate::types::v065::ErrorVerb;
pub use crate::types::v065::HardState;
pub use crate::types::v065::InitialState;
pub use crate::types::v065::LogId;
pub use crate::types::v065::Membership;
pub use crate::types::v065::NodeId;
pub use crate::types::v065::RaftNetwork;
pub use crate::types::v065::RaftStorage;
pub use crate::types::v065::RaftStorageDebug;
pub use crate::types::v065::Snapshot;
pub use crate::types::v065::SnapshotId;
pub use crate::types::v065::SnapshotMeta;
pub use crate::types::v065::SnapshotSegmentId;
pub use crate::types::v065::StateMachineChanges;
pub use crate::types::v065::StorageError;
pub use crate::types::v065::StorageIOError;
pub use crate::types::v065::Violation;
