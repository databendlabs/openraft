#![cfg_attr(feature = "bt", feature(error_generic_member_access))]

#[macro_use]
#[path = "../fixtures/mod.rs"]
mod fixtures;

// The number indicate the preferred running order for these case.
// The later tests may depend on the earlier ones.

mod t10_initialization;
mod t11_shutdown;
mod t50_follower_restart_does_not_interrupt;
mod t50_single_follower_restart;
mod t50_single_leader_restart_re_apply_logs;
mod t90_issue_607_single_restart;
mod t90_issue_920_non_voter_leader_restart;
