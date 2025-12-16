use std::borrow::Borrow;

use crate::LogId;
use crate::RaftState;
use crate::SnapshotMeta;
use crate::StoredMembership;
use crate::engine::EngineConfig;
use crate::engine::LogIdList;
use crate::engine::testing::UTConfig;
use crate::progress::entry::ProgressEntry;
use crate::progress::inflight::Inflight;
use crate::progress::inflight_id::InflightId;
use crate::progress::stream_id::StreamId;
use crate::type_config::alias::LeaderIdOf;
use crate::type_config::alias::LogIdOf;
use crate::vote::RaftLeaderIdExt;

fn log_id(index: u64) -> LogIdOf<UTConfig> {
    LogId {
        leader_id: LeaderIdOf::<UTConfig>::new_committed(1, 1),
        index,
    }
}

fn inflight_logs(prev_index: u64, last_index: u64) -> Inflight<UTConfig> {
    Inflight::logs(Some(log_id(prev_index)), Some(log_id(last_index)), InflightId::new(0))
}

#[test]
fn test_is_log_range_inflight() -> anyhow::Result<()> {
    let mut pe = ProgressEntry::<UTConfig>::empty(StreamId::new(0), 20);
    assert_eq!(false, pe.is_log_range_inflight(&log_id(2)));

    pe.inflight = inflight_logs(2, 4);
    assert_eq!(false, pe.is_log_range_inflight(&log_id(1)));
    assert_eq!(false, pe.is_log_range_inflight(&log_id(2)));
    assert_eq!(true, pe.is_log_range_inflight(&log_id(3)));
    assert_eq!(true, pe.is_log_range_inflight(&log_id(4)));
    assert_eq!(true, pe.is_log_range_inflight(&log_id(5)));

    pe.inflight = Inflight::snapshot(InflightId::new(0));
    assert_eq!(false, pe.is_log_range_inflight(&log_id(5)));

    // LogsSince: all logs after prev are inflight
    pe.inflight = Inflight::logs_since(Some(log_id(2)), InflightId::new(0));
    assert_eq!(false, pe.is_log_range_inflight(&log_id(1)));
    assert_eq!(false, pe.is_log_range_inflight(&log_id(2)));
    assert_eq!(true, pe.is_log_range_inflight(&log_id(3)));
    assert_eq!(true, pe.is_log_range_inflight(&log_id(100)));

    Ok(())
}

#[test]
fn test_update_matching() -> anyhow::Result<()> {
    let engine_config = EngineConfig::new_default(1);

    // Update matching and inflight
    {
        let mut pe = ProgressEntry::<UTConfig>::empty(StreamId::new(0), 20);
        pe.inflight = inflight_logs(5, 10);
        pe.new_updater(&engine_config).update_matching(Some(log_id(6)), Some(InflightId::new(0)));
        assert_eq!(inflight_logs(6, 10), pe.inflight);
        assert_eq!(Some(&log_id(6)), pe.matching());
        assert_eq!(20, pe.searching_end);

        pe.new_updater(&engine_config).update_matching(Some(log_id(10)), Some(InflightId::new(0)));
        assert_eq!(Inflight::None, pe.inflight);
        assert_eq!(Some(&log_id(10)), pe.matching());
        assert_eq!(20, pe.searching_end);
    }

    // `searching_end` should be updated
    {
        let mut pe = ProgressEntry::<UTConfig>::empty(StreamId::new(0), 20);
        pe.matching = Some(log_id(6));
        pe.inflight = inflight_logs(5, 20);

        pe.new_updater(&engine_config).update_matching(Some(log_id(20)), Some(InflightId::new(0)));
        assert_eq!(21, pe.searching_end);
    }

    Ok(())
}

#[test]
fn test_update_conflicting() -> anyhow::Result<()> {
    let mut pe = ProgressEntry::<UTConfig>::empty(StreamId::new(0), 20);
    pe.matching = Some(log_id(3));
    pe.inflight = inflight_logs(5, 10);

    let engine_config = EngineConfig::new_default(1);
    pe.new_updater(&engine_config).update_conflicting(5, Some(InflightId::new(0)));

    assert_eq!(Inflight::None, pe.inflight);
    assert_eq!(&Some(log_id(3)), pe.borrow());
    assert_eq!(5, pe.searching_end);

    Ok(())
}

fn new_raft_state(purge_upto: u64, snap_last: u64, last: u64) -> RaftState<UTConfig> {
    let mut st = RaftState::new(0);

    // TODO: setup log_ids

    st.purge_upto = Some(log_id(purge_upto));
    st.snapshot_meta = SnapshotMeta {
        last_log_id: Some(log_id(snap_last)),
        last_membership: StoredMembership::default(),
        snapshot_id: "".to_string(),
    };

    st.log_ids = LogIdList::new([log_id(purge_upto - 1), log_id(last)]);

    st
}

#[test]
fn test_next_send() -> anyhow::Result<()> {
    // There is already inflight data, return it in an Error
    {
        let mut pe = ProgressEntry::<UTConfig>::empty(StreamId::new(0), 20);
        pe.inflight = inflight_logs(10, 11);
        let res = pe.next_send(&mut new_raft_state(6, 10, 20), 100);
        assert_eq!(Err(&inflight_logs(10, 11)), res);
    }

    {
        //    matching,end
        //    4,5
        //    v
        // -----+------+-----+--->
        //      purged snap  last
        //      6      10    20

        let mut pe = ProgressEntry::<UTConfig>::empty(StreamId::new(0), 4);
        pe.matching = Some(log_id(4));

        let res = pe.next_send(&mut new_raft_state(6, 10, 20), 100);
        assert_eq!(Ok(&Inflight::snapshot(InflightId::new(1))), res);
    }
    {
        //    matching,end
        //    4 6
        //    v-v
        // -----+------+-----+--->
        //      purged snap  last
        //      6      10    20

        let mut pe = ProgressEntry::<UTConfig>::empty(StreamId::new(0), 6);
        pe.matching = Some(log_id(4));

        let res = pe.next_send(&mut new_raft_state(6, 10, 20), 100);
        assert_eq!(Ok(&Inflight::snapshot(InflightId::new(1))), res);
    }

    {
        //    matching,end
        //    4  7
        //    v--v
        // -----+------+-----+--->
        //      purged snap  last
        //      6      10    20

        let mut pe = ProgressEntry::<UTConfig>::empty(StreamId::new(0), 7);
        pe.matching = Some(log_id(4));

        let res = pe.next_send(&mut new_raft_state(6, 10, 20), 100);
        assert_eq!(Ok(&inflight_logs(6, 20).with_id(1)), res);
    }

    {
        //    matching,end
        //    4              20
        //    v--------------v
        // -----+------+-----+--->
        //      purged snap  last
        //      6      10    20

        let mut pe = ProgressEntry::<UTConfig>::empty(StreamId::new(0), 20);
        pe.matching = Some(log_id(4));

        let res = pe.next_send(&mut new_raft_state(6, 10, 20), 100);
        assert_eq!(Ok(&inflight_logs(6, 20).with_id(1)), res);
    }

    //-----------

    {
        //      matching,end
        //      6,7
        //      v
        // -----+------+-----+--->
        //      purged snap  last
        //      6      10    20
        //
        // matching.next_index() == searching_end, enter pipeline mode

        let mut pe = ProgressEntry::<UTConfig>::empty(StreamId::new(0), 7);
        pe.matching = Some(log_id(6));

        let res = pe.next_send(&mut new_raft_state(6, 10, 20), 100);
        assert_eq!(
            Ok(&Inflight::logs_since(Some(log_id(6)), InflightId::new(1))),
            res,
            "pipeline mode: matching.next == searching_end"
        );
    }

    {
        //      matching,end
        //      6, 8
        //      v--v
        // -----+------+-----+--->
        //      purged snap  last
        //      6      10    20

        let mut pe = ProgressEntry::<UTConfig>::empty(StreamId::new(0), 8);
        pe.matching = Some(log_id(6));

        let res = pe.next_send(&mut new_raft_state(6, 10, 20), 100);
        assert_eq!(Ok(&inflight_logs(6, 20).with_id(1)), res);
    }

    {
        //      matching,end
        //      6,           20
        //      v------------v
        // -----+------+-----+--->
        //      purged snap  last
        //      6      10    20

        let mut pe = ProgressEntry::<UTConfig>::empty(StreamId::new(0), 20);
        pe.matching = Some(log_id(6));

        let res = pe.next_send(&mut new_raft_state(6, 10, 20), 100);
        assert_eq!(Ok(&inflight_logs(6, 20).with_id(1)), res);
    }

    {
        //       matching,end
        //       7,          20
        //       v-----------v
        // -----+------+-----+--->
        //      purged snap  last
        //      6      10    20

        let mut pe = ProgressEntry::<UTConfig>::empty(StreamId::new(0), 20);
        pe.matching = Some(log_id(7));

        let res = pe.next_send(&mut new_raft_state(6, 10, 20), 100);
        assert_eq!(Ok(&inflight_logs(7, 20).with_id(1)), res);
    }

    {
        //          matching,end
        //          7,8
        //          v
        // -----+------+-----+--->
        //      purged snap  last
        //      6      10    20
        //
        // matching.next_index() == searching_end, enter pipeline mode

        let mut pe = ProgressEntry::<UTConfig>::empty(StreamId::new(0), 8);
        pe.matching = Some(log_id(7));

        let res = pe.next_send(&mut new_raft_state(6, 10, 20), 100);
        assert_eq!(
            Ok(&Inflight::logs_since(Some(log_id(7)), InflightId::new(1))),
            res,
            "pipeline mode: matching.next == searching_end"
        );
    }

    {
        //                   matching,end
        //                   20,21
        //                   v
        // -----+------+-----+--->
        //      purged snap  last
        //      6      10    20
        //
        // matching.next_index() == searching_end, enter pipeline mode
        // (even though follower is fully caught up)

        let mut pe = ProgressEntry::<UTConfig>::empty(StreamId::new(0), 21);
        pe.matching = Some(log_id(20));

        let res = pe.next_send(&mut new_raft_state(6, 10, 20), 100);
        assert_eq!(
            Ok(&Inflight::logs_since(Some(log_id(20)), InflightId::new(1))),
            res,
            "pipeline mode: fully caught up"
        );
    }

    // Test max_entries
    {
        //       matching,end
        //       7,          20
        //       v-----------v
        // -----+------+-----+--->
        //      purged snap  last
        //      6      10    20

        let mut pe = ProgressEntry::<UTConfig>::empty(StreamId::new(0), 20);
        pe.matching = Some(log_id(7));

        let res = pe.next_send(&mut new_raft_state(6, 10, 20), 5);
        assert_eq!(Ok(&inflight_logs(7, 12).with_id(1)), res);
    }
    Ok(())
}

#[test]
fn test_next_send_pipeline_mode() -> anyhow::Result<()> {
    // Pipeline mode: when matching.next_index() == searching_end,
    // enter streaming mode with LogsSince instead of fixed-range Logs.
    // Basic pipeline mode cases are already covered in test_next_send().
    // This test covers additional edge cases.

    // NOT pipeline mode: matching.next_index() != searching_end
    {
        //       matching  searching_end
        //       7         20
        //       v---------v
        // -----+------+-----+--->
        //      purged snap  last
        //      6      10    20

        let mut pe = ProgressEntry::<UTConfig>::empty(StreamId::new(0), 20);
        pe.matching = Some(log_id(7));
        pe.searching_end = 20;

        let res = pe.next_send(&mut new_raft_state(6, 10, 20), 100);
        assert_eq!(Ok(&inflight_logs(7, 20).with_id(1)), res, "not pipeline: gap exists");
    }

    // NOT pipeline mode: no matching yet
    {
        //  matching=None  searching_end
        //                 20
        //                 v
        // -----+------+-----+--->
        //      purged snap  last
        //      6      10    20

        let mut pe = ProgressEntry::<UTConfig>::empty(StreamId::new(0), 20);
        pe.matching = None;
        pe.searching_end = 20;

        let res = pe.next_send(&mut new_raft_state(6, 10, 20), 100);
        // calc_mid(0, 20) = 10, so start = 10
        assert!(matches!(res, Ok(Inflight::Logs { .. })), "not pipeline: no matching");
    }

    Ok(())
}
