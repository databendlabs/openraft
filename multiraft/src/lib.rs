//! Multi-Raft adapters for connection sharing across Raft groups.
//!
//! - [`GroupedRpc`] - Trait for RPCs with (target, group) routing
//! - [`GroupNetworkAdapter`] - Wraps `GroupedRpc`, implements `RaftNetworkV2`
//! - [`GroupNetworkFactory`] - Simple factory + group_id wrapper
//!
//! See `examples/multi-raft-kv` and `examples/multi-raft-sharding` for usage.

mod network;

pub use network::GroupNetworkAdapter;
pub use network::GroupNetworkFactory;
pub use network::GroupedRpc;
