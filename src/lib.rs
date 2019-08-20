pub mod admin;
mod common;
pub mod config;
mod raft;
mod replication;
pub mod storage;
pub mod messages;
pub mod metrics;
pub mod network;

use std::{error::Error, fmt::Debug};
use serde::{Serialize, de::DeserializeOwned};

pub use crate::raft::Raft;

/// A Raft node's ID.
pub type NodeId = u64;

/// A trait defining application specific data.
///
/// The intention of this trait is that applications which are using this crate will be able to
/// use their own concrete data types throughout their application without having to (de)serialize
/// their data as it goes into Raft. Instead, applications can present their data models as-is to
/// Raft, Raft will present it to the application's `RaftStorage` impl when ready, and the
/// application may then deal with the data directly in the storage engine without having to do a
/// preliminary deserialization.
pub trait AppData: Clone + Debug + Send + Sync + Serialize + DeserializeOwned + 'static {}

/// A trait defining application specific error types.
///
/// The intention of this trait is that applications which are using this crate will be able to
/// pass their own concrete error types up from the storage layer, through the Raft system, to
/// the higher levels of their application for more granular control. Many applications will need
/// to be able to communicate application specific logic from the storage layer.
///
/// **NOTE WELL:** if an `AppError` is returned from any of the `RaftStorage` interfaces, other
/// than the `AppendLogEntry` interface, then the Raft node will immediately shutdown. This is due
/// to the fact that custom error handling logic is only allowed in the `AppendLogEntry` interface
/// while the Raft node is the cluster leader. When the node is in any other state, the storage
/// layer is expected to operate without any errors. Shutting down immediately is how Raft
/// attempts to guard against data corruption and the like.
///
/// At this poinnt in time, `AppError` concrete types are required to implement the serde types
/// for easier integration within parent apps. This may change inn the future depending on how
/// useful this pattern is, or if it ends up just getting in the way.
pub trait AppError: Error + Debug + Send + Sync + Serialize + DeserializeOwned + 'static {}
