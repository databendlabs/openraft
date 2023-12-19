#![cfg_attr(feature = "bt", feature(error_generic_member_access))]

#[macro_use]
#[path = "../fixtures/mod.rs"]
mod fixtures;

// The number indicate the preferred running order for these case.
// See ./README.md

mod t10_client_writes;
mod t11_client_reads;
mod t12_trigger_purge_log;
mod t13_trigger_snapshot;
mod t16_with_raft_state;
mod t50_lagging_network_write;
mod t51_write_when_leader_quit;
