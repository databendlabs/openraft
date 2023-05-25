#![cfg_attr(feature = "bt", feature(error_generic_member_access))]
#![cfg_attr(feature = "bt", feature(provide_any))]

#[macro_use]
#[path = "../fixtures/mod.rs"]
mod fixtures;

// The number indicate the preferred running order for these case.
// See ./README.md

mod t10_client_writes;
mod t11_client_reads;
mod t12_trigger_purge_log;
mod t13_trigger_snapshot;
mod t50_lagging_network_write;
