//! The Raft network interface.

mod backoff;
mod factory;
#[allow(clippy::module_inception)] mod network;
mod rpc_option;
mod rpc_type;
#[cfg(not(feature = "general-snapshot-data"))] pub(crate) mod stream_snapshot;
#[cfg(not(feature = "general-snapshot-data"))] pub(crate) mod streaming;

pub use backoff::Backoff;
pub use factory::RaftNetworkFactory;
pub use network::RaftNetwork;
pub use rpc_option::RPCOption;
pub use rpc_type::RPCTypes;
