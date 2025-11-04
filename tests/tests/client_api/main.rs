#![cfg_attr(feature = "bt", feature(error_generic_member_access))]
#![allow(clippy::uninlined_format_args)]
#[macro_use]
#[path = "../fixtures/mod.rs"]
mod fixtures;

// The number indicate the preferred running order for these case.
// See ./README.md

mod t10_client_writes;
mod t11_client_reads;
mod t12_trigger_purge_log;
mod t13_begin_receiving_snapshot;
mod t13_get_snapshot;
mod t13_install_full_snapshot;
mod t13_trigger_snapshot;
mod t14_transfer_leader;
mod t15_client_write_with_twoshot;
mod t16_with_raft_state;
mod t16_with_state_machine;
mod t50_lagging_network_write;
mod t51_write_when_leader_quit;
