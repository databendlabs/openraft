//! Network V1 adapter for Openraft with chunked snapshot support.
//!
//! This module provides backward compatibility for applications using the v1
//! `RaftNetwork` trait with chunk-based snapshot transmission.
//!
//! # Components
//!
//! ## Client-side Adapter
//!
//! [`Adapter`] wraps any `RaftNetwork` (v1) implementation to provide `RaftNetworkV2`.
//! Use the [`RaftNetwork::into_v2()`] method for convenient conversion.
//!
//! ```ignore
//! use openraft_legacy::network_v1::RaftNetwork;
//!
//! impl RaftNetwork<MyConfig> for MyNetwork {
//!     async fn install_snapshot(...) { ... }  // chunk-based RPC
//!     async fn append_entries(...) { ... }
//!     async fn vote(...) { ... }
//! }
//!
//! // In RaftNetworkFactory:
//! impl RaftNetworkFactory<MyConfig> for MyFactory {
//!     type Network = Adapter<MyConfig, MyNetwork>;
//!
//!     async fn new_client(&mut self, ...) -> Self::Network {
//!         MyNetwork::new(...).into_v2()
//!     }
//! }
//! ```
//!
//! ## Server-side: ChunkedSnapshot
//!
//! [`ChunkedSnapshotReceiver`] is an extension trait that adds `install_snapshot()` to
//! [`openraft::Raft`] for receiving chunks. Just import the trait to use it.
//!
//! ```ignore
//! use openraft_legacy::network_v1::ChunkedSnapshotReceiver;
//!
//! let raft = openraft::Raft::new(...).await?;
//!
//! // Standard Raft methods
//! raft.client_write(...).await?;
//!
//! // Added method for chunked snapshot receiving (via trait)
//! raft.install_snapshot(req).await?;
//! ```

mod adapter;
mod network;
mod sender;
mod snapshot_receiver;
mod streaming;

pub use adapter::Adapter;
pub use network::RaftNetwork;
pub use snapshot_receiver::ChunkedSnapshotReceiver;
