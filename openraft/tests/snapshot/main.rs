#[macro_use]
#[path = "../fixtures/mod.rs"]
mod fixtures;

mod t20_api_install_snapshot;
mod t20_trigger_snapshot;
mod t23_snapshot_chunk_size;
mod t24_snapshot_when_lacking_log;
mod t25_snapshot_line_rate_to_snapshot;
mod t40_after_snapshot_add_learner_and_request_a_log;
mod t40_purge_in_snapshot_logs;
mod t41_snapshot_overrides_membership;
mod t42_snapshot_uses_prev_snap_membership;
mod t43_snapshot_delete_conflict_logs;
