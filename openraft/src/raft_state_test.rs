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

    assert!(rs.has_log_id(None));
    assert!(!rs.has_log_id(Some(log_id(0, 0))));

    Ok(())
}

#[test]
fn test_raft_state_has_log_id_committed_gets_true() -> anyhow::Result<()> {
    let rs = RaftState {
        committed: Some(log_id(2, 1)),
        ..Default::default()
    };

    assert!(rs.has_log_id(None));
    assert!(rs.has_log_id(Some(log_id(0, 0))));
    assert!(rs.has_log_id(Some(log_id(2, 1))));
    assert!(!rs.has_log_id(Some(log_id(2, 2))));

    Ok(())
}

#[test]
fn test_raft_state_has_log_id_in_log_id_list() -> anyhow::Result<()> {
    let rs = RaftState {
        committed: Some(log_id(2, 1)),
        log_ids: LogIdList::new(vec![log_id(1, 2), log_id(3, 4)]),
        ..Default::default()
    };

    assert!(rs.has_log_id(None));
    assert!(rs.has_log_id(Some(log_id(0, 0))));
    assert!(rs.has_log_id(Some(log_id(2, 1))));
    assert!(rs.has_log_id(Some(log_id(1, 3))));
    assert!(rs.has_log_id(Some(log_id(3, 4))));

    assert!(!rs.has_log_id(Some(log_id(2, 3))));
    assert!(!rs.has_log_id(Some(log_id(2, 4))));
    assert!(!rs.has_log_id(Some(log_id(3, 5))));

    Ok(())
}
