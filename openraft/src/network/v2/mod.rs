#[cfg(all(feature = "tokio-rt", feature = "adapt-network-v1"))]
mod adapt_v1;
mod network;

pub use network::RaftNetworkV2;
