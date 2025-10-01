use std::time::Duration;

use crate::RaftState;
use crate::Vote;
use crate::engine::LogIdList;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::type_config::TypeConfigExt;
use crate::utime::Leased;

#[test]
fn test_is_initialized() {
    // empty
    {
        let rs = RaftState::<UTConfig> { ..Default::default() };

        assert_eq!(false, rs.is_initialized());
    }

    // Vote is set but is default
    {
        let rs = RaftState::<UTConfig> {
            vote: Leased::new(UTConfig::<()>::now(), Duration::from_millis(500), Vote::default()),
            ..Default::default()
        };

        assert_eq!(false, rs.is_initialized());
    }

    // Vote is non-default value
    {
        let rs = RaftState::<UTConfig> {
            vote: Leased::new(UTConfig::<()>::now(), Duration::from_millis(500), Vote::new(1, 2)),
            ..Default::default()
        };

        assert_eq!(true, rs.is_initialized());
    }

    // Logs are non-empty
    {
        let rs = RaftState::<UTConfig> {
            log_ids: LogIdList::new([log_id(0, 0, 0)]),
            ..Default::default()
        };

        assert_eq!(true, rs.is_initialized());
    }
}
