use validit::Validate;

use crate::engine::testing::UTConfig;
use crate::engine::LogIdList;
use crate::storage::SnapshotMeta;
use crate::CommittedLeaderId;
use crate::LogId;
use crate::RaftState;

fn log_id(term: u64, index: u64) -> LogId<UTConfig> {
    LogId {
        leader_id: CommittedLeaderId::new(term, 0),
        index,
    }
}

#[test]
fn test_raft_state_validate_snapshot_is_none() -> anyhow::Result<()> {
    // Some app does not persist snapshot, when restarted, purged is not None but snapshot_last_log_id
    // is None. This is a valid state and should not emit error.

    let rs = RaftState::<UTConfig> {
        log_ids: LogIdList::new(vec![log_id(1, 1), log_id(3, 4)]),
        purged_next: 2,
        purge_upto: Some(log_id(1, 1)),
        committed: Some(log_id(1, 1)),
        snapshot_meta: SnapshotMeta::default(),
        ..Default::default()
    };

    assert!(rs.validate().is_ok());

    Ok(())
}
