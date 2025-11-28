//! Multi-Raft support: adapters for connection sharing across groups.
//!
//! - [`GroupedRpc`] - Trait for RPCs with group routing
//! - [`GroupNetworkAdapter`] - Wraps `GroupedRpc`, implements `RaftNetworkV2`
//! - [`GroupNetworkFactory`] - Simple factory + group_id wrapper
//!
//! See `examples/multi-raft-kv` and `examples/multi-raft-sharding` for usage.

mod network;

pub use network::GroupNetworkAdapter;
pub use network::GroupNetworkFactory;
pub use network::GroupedRpc;
