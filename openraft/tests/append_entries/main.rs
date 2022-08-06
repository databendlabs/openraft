#![cfg_attr(feature = "bt", feature(backtrace))]

#[macro_use]
#[path = "../fixtures/mod.rs"]
mod fixtures;

// The number indicate the preferred running order for these case.
// The later tests may depend on the earlier ones.

mod t10_conflict_with_empty_entries;
mod t10_see_higher_vote;
mod t20_append_conflicts;
mod t30_append_inconsistent_log;
mod t40_append_updates_membership;
mod t50_append_entries_with_bigger_term;
mod t50_replication_1_voter_to_isolated_learner;
mod t60_enable_heartbeat;
mod t60_large_heartbeat;
mod t90_issue_216_stale_last_log_id;
