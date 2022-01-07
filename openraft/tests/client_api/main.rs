#[macro_use]
#[path = "../fixtures/mod.rs"]
mod fixtures;

// The number indicate the preferred running order for these case.
// The later tests may depend on the earlier ones.

mod t10_client_writes;
mod t20_client_reads;
mod t50_lagging_network_write;
