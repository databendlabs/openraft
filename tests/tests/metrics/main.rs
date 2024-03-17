#![cfg_attr(feature = "bt", feature(error_generic_member_access))]

#[macro_use]
#[path = "../fixtures/mod.rs"]
mod fixtures;

// The number indicate the preferred running order for these case.
// The later tests may depend on the earlier ones.

mod t10_current_leader;
mod t10_leader_last_ack;
mod t10_purged;
mod t10_server_metrics_and_data_metrics;
mod t20_metrics_state_machine_consistency;
mod t30_leader_metrics;
mod t40_metrics_wait;
