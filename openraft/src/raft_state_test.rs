use crate::engine::LogIdList;
use crate::LeaderId;
use crate::LogId;
use crate::RaftState;

fn log_id(term: u64, index: u64) -> LogId<u64> {
    LogId::<u64> {
        leader_id: LeaderId { term, node_id: 1 },
        index,
    }
}

#[test]
fn test_raft_state_has_log_id_empty() -> anyhow::Result<()> {
    let rs = RaftState::default();

    assert!(!rs.has_log_id(&log_id(0, 0)));

    Ok(())
}

#[test]
fn test_raft_state_has_log_id_committed_gets_true() -> anyhow::Result<()> {
    let rs = RaftState {
        committed: Some(log_id(2, 1)),
        ..Default::default()
    };

    assert!(rs.has_log_id(&log_id(0, 0)));
    assert!(rs.has_log_id(&log_id(2, 1)));
    assert!(!rs.has_log_id(&log_id(2, 2)));

    Ok(())
}

#[test]
fn test_raft_state_has_log_id_in_log_id_list() -> anyhow::Result<()> {
    let rs = RaftState {
        committed: Some(log_id(2, 1)),
        log_ids: LogIdList::new(vec![log_id(1, 2), log_id(3, 4)]),
        ..Default::default()
    };

    assert!(rs.has_log_id(&log_id(0, 0)));
    assert!(rs.has_log_id(&log_id(2, 1)));
    assert!(rs.has_log_id(&log_id(1, 3)));
    assert!(rs.has_log_id(&log_id(3, 4)));

    assert!(!rs.has_log_id(&log_id(2, 3)));
    assert!(!rs.has_log_id(&log_id(2, 4)));
    assert!(!rs.has_log_id(&log_id(3, 5)));

    Ok(())
}

#[test]
fn test_raft_state_last_log_id() -> anyhow::Result<()> {
    let rs = RaftState::<u64> {
        log_ids: LogIdList::new(vec![]),
        ..Default::default()
    };

    assert_eq!(None, rs.last_log_id());

    let rs = RaftState {
        log_ids: LogIdList::new(vec![log_id(1, 2)]),
        ..Default::default()
    };
    assert_eq!(Some(log_id(1, 2)), rs.last_log_id());

    let rs = RaftState {
        log_ids: LogIdList::new(vec![log_id(1, 2), log_id(3, 4)]),
        ..Default::default()
    };
    assert_eq!(Some(log_id(3, 4)), rs.last_log_id());

    Ok(())
}

#[test]
fn test_raft_state_last_purged_log_id() -> anyhow::Result<()> {
    let rs = RaftState::<u64> {
        log_ids: LogIdList::new(vec![]),
        ..Default::default()
    };

    assert_eq!(None, rs.last_purged_log_id());

    let rs = RaftState {
        log_ids: LogIdList::new(vec![log_id(1, 2)]),
        ..Default::default()
    };
    assert_eq!(Some(log_id(1, 2)), rs.last_purged_log_id());

    let rs = RaftState {
        log_ids: LogIdList::new(vec![log_id(1, 2), log_id(3, 4)]),
        ..Default::default()
    };
    assert_eq!(Some(log_id(1, 2)), rs.last_purged_log_id());

    Ok(())
}
