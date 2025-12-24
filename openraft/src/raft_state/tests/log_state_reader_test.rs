use crate::RaftState;
use crate::engine::LogIdList;
use crate::engine::testing::UTConfig;
use crate::raft_state::LogStateReader;
use crate::type_config::alias::LogIdOf;

fn log_id(term: u64, index: u64) -> LogIdOf<UTConfig> {
    crate::engine::testing::log_id(term, 0, index)
}

#[test]
fn test_raft_state_prev_log_id() -> anyhow::Result<()> {
    // There is log id at 0
    // Last-per-leader: leader 0 at 0, leader 1 at 1-3, leader 3 at 4
    {
        let rs = RaftState::<UTConfig> {
            log_ids: LogIdList::new(None, vec![log_id(0, 0), log_id(1, 3), log_id(3, 4)]),
            ..Default::default()
        };

        assert_eq!(None, rs.prev_log_id(0));
        assert_eq!(Some(log_id(0, 0)), rs.prev_log_id(1));
        assert_eq!(Some(log_id(1, 3)), rs.prev_log_id(4));
        assert_eq!(Some(log_id(3, 4)), rs.prev_log_id(5));
    }

    // First log at index 1 (purged at 0)
    // Last-per-leader: leader 1 at 1-3, leader 3 at 4
    {
        let rs = RaftState::<UTConfig> {
            log_ids: LogIdList::new(Some(log_id(0, 0)), vec![log_id(1, 3), log_id(3, 4)]),
            purged_next: 1,
            ..Default::default()
        };

        assert_eq!(None, rs.prev_log_id(0));
        assert_eq!(Some(log_id(0, 0)), rs.prev_log_id(1));
        assert_eq!(Some(log_id(1, 1)), rs.prev_log_id(2));
        assert_eq!(Some(log_id(1, 3)), rs.prev_log_id(4));
        assert_eq!(Some(log_id(3, 4)), rs.prev_log_id(5));
    }
    Ok(())
}

#[test]
fn test_raft_state_has_log_id_empty() -> anyhow::Result<()> {
    let rs = RaftState::<UTConfig>::default();

    assert!(!rs.has_log_id(log_id(0, 0)));

    Ok(())
}

#[test]
fn test_raft_state_has_log_id_committed_gets_true() -> anyhow::Result<()> {
    let mut rs = RaftState::<UTConfig>::default();

    rs.apply_progress_mut().accept(log_id(2, 1));

    assert!(rs.has_log_id(log_id(0, 0)));
    assert!(rs.has_log_id(log_id(2, 1)));
    assert!(!rs.has_log_id(log_id(2, 2)));

    Ok(())
}

#[test]
fn test_raft_state_has_log_id_in_log_id_list() -> anyhow::Result<()> {
    // Last-per-leader: leader 1 at 0-3, leader 3 at 4
    let mut rs = RaftState::<UTConfig> {
        log_ids: LogIdList::new(None, vec![log_id(1, 3), log_id(3, 4)]),
        ..Default::default()
    };

    rs.apply_progress_mut().accept(log_id(2, 1));

    assert!(rs.has_log_id(log_id(1, 0)));
    assert!(rs.has_log_id(log_id(2, 1)));
    assert!(rs.has_log_id(log_id(1, 3)));
    assert!(rs.has_log_id(log_id(3, 4)));

    assert!(!rs.has_log_id(log_id(2, 3)));
    assert!(!rs.has_log_id(log_id(2, 4)));
    assert!(!rs.has_log_id(log_id(3, 5)));

    Ok(())
}

#[test]
fn test_raft_state_last_log_id() -> anyhow::Result<()> {
    let rs = RaftState::<UTConfig> {
        log_ids: LogIdList::new(None, vec![]),
        ..Default::default()
    };

    assert_eq!(None, rs.last_log_id());

    let rs = RaftState::<UTConfig> {
        log_ids: LogIdList::new(None, vec![log_id(1, 2)]),
        ..Default::default()
    };
    assert_eq!(Some(&log_id(1, 2)), rs.last_log_id());

    let rs = RaftState::<UTConfig> {
        log_ids: LogIdList::new(None, vec![log_id(1, 2), log_id(3, 4)]),
        ..Default::default()
    };
    assert_eq!(Some(&log_id(3, 4)), rs.last_log_id());

    Ok(())
}

#[test]
fn test_raft_state_purge_upto() -> anyhow::Result<()> {
    let rs = RaftState::<UTConfig> {
        purge_upto: Some(log_id(1, 2)),
        ..Default::default()
    };

    assert_eq!(Some(&log_id(1, 2)), rs.purge_upto());

    Ok(())
}

#[test]
fn test_raft_state_last_purged_log_id() -> anyhow::Result<()> {
    // No purged logs
    let rs = RaftState::<UTConfig> {
        log_ids: LogIdList::new(None, vec![]),
        ..Default::default()
    };
    assert_eq!(None, rs.last_purged_log_id());

    // Purged at index 2 (with purged_next = 3)
    let rs = RaftState::<UTConfig> {
        log_ids: LogIdList::new(Some(log_id(1, 2)), vec![log_id(3, 4)]),
        purged_next: 3,
        ..Default::default()
    };
    assert_eq!(Some(log_id(1, 2)), rs.last_purged_log_id().copied());

    // Purged at index 2, logs at 3-4
    let rs = RaftState::<UTConfig> {
        log_ids: LogIdList::new(Some(log_id(1, 2)), vec![log_id(3, 4)]),
        purged_next: 3,
        ..Default::default()
    };
    assert_eq!(Some(log_id(1, 2)), rs.last_purged_log_id().copied());

    Ok(())
}
