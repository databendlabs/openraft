#![cfg_attr(feature="docinclude", feature(external_doc))]
#![cfg_attr(feature="docinclude", doc(include="../README.md"))]

pub mod admin;
mod common;
pub mod config;
pub mod messages;
pub mod metrics;
pub mod network;
mod raft;
mod replication;
pub mod storage;

use std::{error::Error, fmt::Debug};
use serde::{Serialize, de::DeserializeOwned};

// Top-level exports.
pub use crate::{
    config::{Config, ConfigBuilder, SnapshotPolicy},
    raft::Raft,
    metrics::RaftMetrics,
    network::RaftNetwork,
    storage::RaftStorage,
};

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
pub trait AppData: Clone + Debug + Send + Sync + Serialize + DeserializeOwned + 'static {}

/// A trait defining application specific response data.
///
/// The intention of this trait is that applications which are using this crate will be able to
/// use their own concrete data types for returning response data from the storage layer when an
/// entry is successfully applied to the state machine as part of a client request (this is not
/// used during replication). This allows applications to seamlessly return application specific
/// data from their storage layer, up through Raft, and back into their application for returning
/// data to clients or other such uses.
pub trait AppDataResponse: Clone + Debug + Send + Sync + Serialize + DeserializeOwned + 'static {}

/// A trait defining application specific error types.
///
/// The intention of this trait is that applications which are using this crate will be able to
/// pass their own concrete error types up from the storage layer, through the Raft system, to
/// the higher levels of their application for more granular control. Many applications will need
/// to be able to communicate application specific logic from the storage layer.
///
/// **NOTE WELL:** if an `AppError` is returned from any of the `RaftStorage` interfaces, other
/// than the `AppendEntryToLog` interface, then the Raft node will immediately shutdown. This is due
/// to the fact that custom error handling logic is only allowed in the `AppendEntryToLog` interface
/// while the Raft node is the cluster leader. When the node is in any other state, the storage
/// layer is expected to operate without any errors. Shutting down immediately is how Raft
/// attempts to guard against data corruption and the like.
///
/// At this point in time, `AppError` concrete types are required to implement the serde types
/// for easier integration within parent apps. This may change in the future depending on how
/// useful this pattern is, or if it ends up just getting in the way.
pub trait AppError: Error + Debug + Send + Sync + Serialize + DeserializeOwned + 'static {}
