use validit::Validate;

use crate::RaftState;
use crate::engine::LogIdList;
use crate::engine::testing::UTConfig;
use crate::storage::SnapshotMeta;
use crate::type_config::alias::LogIdOf;

fn log_id(term: u64, index: u64) -> LogIdOf<UTConfig> {
    crate::engine::testing::log_id(term, 0, index)
}

#[test]
fn test_raft_state_validate_snapshot_is_none() -> anyhow::Result<()> {
    // Some app does not persist snapshot, when restarted, purged is not None but snapshot_last_log_id
    // is None. This is a valid state and should not emit error.

    let mut rs = RaftState::<UTConfig> {
        log_ids: LogIdList::new(vec![log_id(1, 1), log_id(3, 4)]),
        purged_next: 2,
        purge_upto: Some(log_id(1, 1)),
        snapshot_meta: SnapshotMeta::default(),
        ..Default::default()
    };

    rs.apply_progress_mut().accept(log_id(1, 1));

    assert!(rs.validate().is_ok());

    Ok(())
}
