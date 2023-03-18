#![cfg_attr(feature = "bt", feature(error_generic_member_access))]
#![cfg_attr(feature = "bt", feature(provide_any))]

#[macro_use]
#[path = "../fixtures/mod.rs"]
mod fixtures;

// The number indicate the preferred running order for these case.
// The later tests may depend on the earlier ones.

mod t20_initialization;
mod t20_shutdown;
mod t30_follower_restart_does_not_interrupt;
mod t30_single_follower_restart;
mod t90_issue_607_single_restart;
