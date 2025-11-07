#![cfg_attr(feature = "bt", feature(error_generic_member_access))]
#![allow(clippy::uninlined_format_args)]
#[macro_use]
#[path = "../fixtures/mod.rs"]
mod fixtures;

mod t10_append_entries_partial_success;
mod t50_append_entries_backoff;
mod t50_append_entries_backoff_rejoin;
mod t60_feature_loosen_follower_log_revert;
mod t61_allow_follower_log_revert;
mod t62_follower_clear_restart_recover;
