#![cfg_attr(feature = "bt", feature(error_generic_member_access))]
#![cfg_attr(feature = "bt", feature(provide_any))]

#[macro_use]
#[path = "../fixtures/mod.rs"]
mod fixtures;

mod t10_append_entries_partial_success;
mod t50_append_entries_backoff;
mod t50_append_entries_backoff_rejoin;
#[cfg(feature = "loosen-follower-log-revert")]
mod t60_feature_loosen_follower_log_revert;
