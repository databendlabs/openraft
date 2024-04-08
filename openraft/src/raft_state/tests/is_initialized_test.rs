use crate::engine::LogIdList;
use crate::testing::log_id;
use crate::utime::UTime;
use crate::RaftState;
use crate::TokioInstant;
use crate::Vote;

#[test]
fn test_is_initialized() {
    // empty
    {
        let rs = RaftState::<u64, (), TokioInstant> { ..Default::default() };

        assert_eq!(false, rs.is_initialized());
    }

    // Vote is set but is default
    {
        let rs = RaftState::<u64, (), TokioInstant> {
            vote: UTime::new(TokioInstant::now(), Vote::default()),
            ..Default::default()
        };

        assert_eq!(false, rs.is_initialized());
    }

    // Vote is non-default value
    {
        let rs = RaftState::<u64, (), TokioInstant> {
            vote: UTime::new(TokioInstant::now(), Vote::new(1, 2)),
            ..Default::default()
        };

        assert_eq!(true, rs.is_initialized());
    }

    // Logs are non-empty
    {
        let rs = RaftState::<u64, (), TokioInstant> {
            log_ids: LogIdList::new([log_id(0, 0, 0)]),
            ..Default::default()
        };

        assert_eq!(true, rs.is_initialized());
    }
}
