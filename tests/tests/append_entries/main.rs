#![cfg_attr(feature = "bt", feature(error_generic_member_access))]

#[macro_use]
#[path = "../fixtures/mod.rs"]
mod fixtures;

// The number indicate the preferred running order for these case.
// The later tests may depend on the earlier ones.

mod t10_conflict_with_empty_entries;
mod t10_see_higher_vote;
mod t11_append_conflicts;
mod t11_append_entries_with_bigger_term;
mod t11_append_inconsistent_log;
mod t11_append_updates_membership;
mod t30_replication_1_voter_to_isolated_learner;
mod t60_enable_heartbeat;
mod t61_heartbeat_reject_vote;
mod t61_large_heartbeat;
mod t90_issue_216_stale_last_log_id;
