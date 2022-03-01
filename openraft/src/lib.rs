#![doc = include_str!("../README.md")]
#![feature(backtrace)]

mod config;
mod core;
mod defensive;
mod membership;
mod node;
mod raft_types;
mod replication;
mod storage_error;
mod store_ext;
mod store_wrapper;
mod summary;
mod vote;

pub mod error;
pub mod metrics;
pub mod network;
pub mod raft;
pub mod storage;
pub mod testing;

#[cfg(test)]
mod metrics_wait_test;

pub use async_trait;
use serde::de::DeserializeOwned;
use serde::Serialize;

pub use crate::config::Config;
pub use crate::config::ConfigError;
pub use crate::config::SnapshotPolicy;
pub use crate::core::EffectiveMembership;
pub use crate::core::State;
pub use crate::defensive::DefensiveCheck;
pub use crate::defensive::DefensiveCheckBase;
pub use crate::membership::Membership;
pub use crate::metrics::RaftMetrics;
pub use crate::network::RPCTypes;
pub use crate::network::RaftNetwork;
pub use crate::network::RaftNetworkFactory;
pub use crate::node::Node;
pub use crate::node::NodeId;
pub use crate::raft::Raft;
pub use crate::raft::RaftTypeConfig;
pub use crate::raft_types::LogId;
pub use crate::raft_types::LogIdOptionExt;
pub use crate::raft_types::SnapshotId;
pub use crate::raft_types::SnapshotSegmentId;
pub use crate::raft_types::StateMachineChanges;
pub use crate::raft_types::Update;
pub use crate::replication::ReplicationMetrics;
pub use crate::storage::RaftLogReader;
pub use crate::storage::RaftSnapshotBuilder;
pub use crate::storage::RaftStorage;
pub use crate::storage::RaftStorageDebug;
pub use crate::storage::SnapshotMeta;
pub use crate::storage_error::DefensiveError;
pub use crate::storage_error::ErrorSubject;
pub use crate::storage_error::ErrorVerb;
pub use crate::storage_error::StorageError;
pub use crate::storage_error::StorageIOError;
pub use crate::storage_error::ToStorageResult;
pub use crate::storage_error::Violation;
pub use crate::store_ext::StoreExt;
pub use crate::store_wrapper::Wrapper;
pub use crate::summary::MessageSummary;
pub use crate::vote::LeaderId;
pub use crate::vote::Vote;

/// A trait defining application specific data.
///
/// The intention of this trait is that applications which are using this crate will be able to
/// use their own concrete data types throughout their application without having to serialize and
/// deserialize their data as it goes through Raft. Instead, applications can present their data
/// models as-is to Raft, Raft will present it to the application's `RaftStorage` impl when ready,
/// and the application may then deal with the data directly in the storage engine without having
/// to do a preliminary deserialization.
///
/// ## Note
///
/// The trait is automatically implemented for all types which satisfy its supertraits.
pub trait AppData: Clone + Send + Sync + Serialize + DeserializeOwned + 'static {}

impl<T> AppData for T where T: Clone + Send + Sync + Serialize + DeserializeOwned + 'static {}

/// A trait defining application specific response data.
///
/// The intention of this trait is that applications which are using this crate will be able to
/// use their own concrete data types for returning response data from the storage layer when an
/// entry is applied to the state machine as part of a client request (this is not used during
/// replication). This allows applications to seamlessly return application specific data from
/// their storage layer, up through Raft, and back into their application for returning
/// data to clients.
///
/// This type must encapsulate both success and error responses, as application specific logic
/// related to the success or failure of a client request — application specific validation logic,
/// enforcing of data constraints, and anything of that nature — are expressly out of the realm of
/// the Raft consensus protocol.
///
/// ## Note
///
/// The trait is automatically implemented for all types which satisfy its supertraits.
pub trait AppDataResponse: Clone + Send + Sync + Serialize + DeserializeOwned + 'static {}

impl<T> AppDataResponse for T where T: Clone + Send + Sync + Serialize + DeserializeOwned + 'static {}
