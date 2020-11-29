#![cfg_attr(feature = "docinclude", feature(external_doc))]
#![cfg_attr(feature = "docinclude", doc(include = "../README.md"))]

pub mod config;
mod core;
pub mod error;
pub mod metrics;
pub mod network;
pub mod raft;
mod replication;
pub mod storage;

use serde::{de::DeserializeOwned, Serialize};

pub use crate::{
    config::{Config, ConfigBuilder, SnapshotPolicy},
    core::State,
    error::{ChangeConfigError, ClientWriteError, ConfigError, InitializeError, RaftError},
    metrics::RaftMetrics,
    network::RaftNetwork,
    raft::Raft,
    storage::RaftStorage,
};
pub use async_trait;

/// A Raft node's ID.
pub type NodeId = u64;

/// A trait defining application specific data.
///
/// The intention of this trait is that applications which are using this crate will be able to
/// use their own concrete data types throughout their application without having to serialize and
/// deserialize their data as it goes through Raft. Instead, applications can present their data
/// models as-is to Raft, Raft will present it to the application's `RaftStorage` impl when ready,
/// and the application may then deal with the data directly in the storage engine without having
/// to do a preliminary deserialization.
pub trait AppData: Clone + Send + Sync + Serialize + DeserializeOwned + 'static {}

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
pub trait AppDataResponse: Clone + Send + Sync + Serialize + DeserializeOwned + 'static {}
