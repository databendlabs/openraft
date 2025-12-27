//! RaftNetwork V1 adapter for Openraft with chunked snapshot support.
//!
//! This crate provides backward compatibility for applications using the v1
//! `RaftNetwork` trait with chunk-based snapshot transmission.
//!
//! # Components
//!
//! ## Client-side Adapter
//!
//! [`Adapter`] wraps any `RaftNetwork` (v1) implementation to provide `RaftNetworkV2`.
//!
//! ```ignore
//! use openraft_network_v1::{Adapter, RaftNetwork};
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
//!         Adapter::new(MyNetwork::new(...))
//!     }
//! }
//! ```
//!
//! ## Server-side: ChunkedRaft
//!
//! [`ChunkedRaft`] wraps [`openraft::Raft`] and adds `install_snapshot()` for receiving
//! chunks. It derefs to the inner Raft, so all standard methods are available.
//!
//! ```ignore
//! use openraft_network_v1::ChunkedRaft;
//!
//! let inner = openraft::Raft::new(...).await?;
//! let raft = ChunkedRaft::new(inner);
//!
//! // Standard Raft methods via Deref
//! raft.client_write(...).await?;
//!
//! // Added method for chunked snapshot receiving
//! raft.install_snapshot(req).await?;
//! ```

mod adapt_v1_to_v2;
mod chunked_raft;
mod network;
mod receiver;
mod sender;
mod streaming;

pub use adapt_v1_to_v2::Adapter;
pub use chunked_raft::ChunkedRaft;
pub use network::RaftNetwork;
