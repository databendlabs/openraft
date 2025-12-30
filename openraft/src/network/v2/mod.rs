//! Version 2 of the Raft network API.
//!
//! [`RaftNetworkV2`] is the full network interface that combines multiple
//! independent network feature traits into a single interface.

mod network;

pub use network::RaftNetworkV2;
