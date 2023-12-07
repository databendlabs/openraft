use crate::engine::LogIdList;
use crate::utime::UTime;
use crate::CommittedLeaderId;
use crate::LogId;
use crate::RaftState;
use crate::TokioInstant;
use crate::Vote;

fn log_id(term: u64, index: u64) -> LogId<u64> {
    LogId::<u64> {
        leader_id: CommittedLeaderId::new(term, 0),
        index,
    }
}

#[test]
fn test_raft_state_get_read_log_id() -> anyhow::Result<()> {
    let log_ids = || LogIdList::new(vec![log_id(1, 1), log_id(3, 4), log_id(3, 6)]);
    {
        let rs = RaftState::<u64, (), TokioInstant> {
            vote: UTime::without_utime(Vote::new_committed(3, 0)),
            log_ids: log_ids(),
            committed: Some(log_id(2, 1)),
            ..Default::default()
        };

        assert_eq!(Some(log_id(3, 4)), rs.get_read_log_id().copied());
    }

    {
        let rs = RaftState::<u64, (), TokioInstant> {
            vote: UTime::without_utime(Vote::new_committed(3, 0)),
            log_ids: log_ids(),
            committed: Some(log_id(3, 5)),
            ..Default::default()
        };
        assert_eq!(Some(log_id(3, 5)), rs.get_read_log_id().copied());
    }
    Ok(())
}
