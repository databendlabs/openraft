//! RaftNetwork V1 adapter for Openraft with chunked snapshot support.
//!
//! This crate provides backward compatibility for applications using the v1
//! `RaftNetwork` trait with chunk-based snapshot transmission.
//!
//! # Components
//!
//! ## Client-side Adapter
//!
//! The blanket implementation in [`adapt_v1_to_v2`] automatically converts any
//! `RaftNetwork` (v1) implementation to `RaftNetworkV2`. This allows existing
//! network implementations to work with the new API without modification.
//!
//! ```ignore
//! use openraft::RaftNetwork;
//! use openraft_network_v1::adapt_v1_to_v2;  // Import to enable blanket impl
//!
//! impl RaftNetwork<MyConfig> for MyNetwork {
//!     async fn install_snapshot(...) { ... }  // chunk-based RPC
//!     async fn append_entries(...) { ... }
//!     async fn vote(...) { ... }
//! }
//! // MyNetwork now automatically implements RaftNetworkV2
//! ```
//!
//! ## Server-side: Extended Raft
//!
//! [`RaftV1`] wraps [`openraft::Raft`] and adds `install_snapshot()` for receiving
//! chunks. It derefs to the inner Raft, so all standard methods are available.
//!
//! ```ignore
//! use openraft_network_v1::RaftV1;
//!
//! let inner = openraft::Raft::new(...).await?;
//! let raft = RaftV1::new(inner);
//!
//! // Standard Raft methods via Deref
//! raft.client_write(...).await?;
//!
//! // Added method for v1 protocol
//! raft.install_snapshot(req).await?;
//! ```
//!
//! ## Low-level Components
//!
//! - [`Chunked`] - Send and receive snapshot chunks
//! - [`Streaming`] - Per-transfer receiving state
//! - [`StreamingState`] - Shared state for `Extensions` storage

pub mod adapt_v1_to_v2;
pub mod chunked;
pub mod network;
pub mod raft;
pub mod streaming;

pub use adapt_v1_to_v2::Adapter;
pub use chunked::Chunked;
pub use network::RaftNetwork;
pub use raft::RaftV1;
pub use streaming::Streaming;
pub use streaming::StreamingState;
