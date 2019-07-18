pub mod config;
mod raft;
mod replication;
pub mod storage;
pub mod memory_storage;
pub mod messages;
pub mod metrics;
pub mod network;

use std::fmt::Debug;
use serde::{Serialize, de::DeserializeOwned};

pub use crate::raft::Raft;

/// A Raft node's ID.
pub type NodeId = u64;

/// A trait defining application specific error types.
///
/// The intention of this trait is that applications which are using this crate will be able to
/// pass their own concrete error types up from the storage layer, through the Raft system, to
/// the higher levels of their application for more granular control; or from the network layer,
/// when Raft needs to forward client requests to the Raft leader. Many applications will need to
/// be able to communicate application specific logic from the storage layer.
///
/// **NOTE WELL:** if an `AppError` is returned from any of the `RaftStorage` interfaces, other
/// than the `AppendLogEntries` interface while the Raft node is in **leader state**, then the Raft
/// node will shutdown. This is due to the fact that custom error handling logic is only allowed
/// in the `AppendLogEntries` interface while the Raft node is the cluster leader. When the node
/// is in any other state, the storage layer is expected to operate without any errors.
///
/// When Raft needs to forward a client request via the `RaftNetwork` interface, any `AppError`
/// instances will simply be passed back through to the original call point.
///
/// It is also important to note that there is a case where the custom error type being used here
/// will need to be serialized and sent over the wire. The `RaftNetwork` trait must be able to
/// handle the `ClientPayload` message type, which is used for forwarding client requests
/// to the Raft leader, and its `Result::Err` type contains a `AppError` variant. **The parent
/// application is responsible for serializing and deserializing thier `AppError` type.** This
/// will need to be handled in the `RaftNetwork` implementation. The serde traits are here to
/// help with that.
pub trait AppError: Debug + Send + Sync + Serialize + DeserializeOwned + 'static {}
