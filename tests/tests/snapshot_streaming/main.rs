#![cfg_attr(feature = "bt", feature(error_generic_member_access))]

#[macro_use]
#[path = "../fixtures/mod.rs"]
mod fixtures;

mod t10_api_install_snapshot;
mod t20_startup_snapshot;
mod t30_purge_in_snapshot_logs;
mod t31_snapshot_overrides_membership;
mod t32_snapshot_uses_prev_snap_membership;
mod t33_snapshot_delete_conflict_logs;
mod t34_replication_does_not_block_purge;
mod t50_snapshot_line_rate_to_snapshot;
mod t50_snapshot_when_lacking_log;
mod t51_after_snapshot_add_learner_and_request_a_log;
mod t60_snapshot_chunk_size;
mod t90_issue_808_snapshot_to_unreachable_node_should_not_block;
