use std::borrow::Borrow;

use crate::LogId;
use crate::engine::EngineConfig;
use crate::engine::testing::UTConfig;
use crate::log_id::ref_log_id::RefLogId;
use crate::progress::entry::ProgressEntry;
use crate::progress::inflight::Inflight;
use crate::raft_state::LogStateReader;
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
    Inflight::logs(Some(log_id(prev_index)), Some(log_id(last_index)))
}

#[test]
fn test_is_log_range_inflight() -> anyhow::Result<()> {
    let mut pe = ProgressEntry::<UTConfig>::empty(20);
    assert_eq!(false, pe.is_log_range_inflight(&log_id(2)));

    pe.inflight = inflight_logs(2, 4);
    assert_eq!(false, pe.is_log_range_inflight(&log_id(1)));
    assert_eq!(false, pe.is_log_range_inflight(&log_id(2)));
    assert_eq!(true, pe.is_log_range_inflight(&log_id(3)));
    assert_eq!(true, pe.is_log_range_inflight(&log_id(4)));
    assert_eq!(true, pe.is_log_range_inflight(&log_id(5)));

    pe.inflight = Inflight::snapshot();
    assert_eq!(false, pe.is_log_range_inflight(&log_id(5)));

    Ok(())
}

#[test]
fn test_update_matching() -> anyhow::Result<()> {
    let engine_config = EngineConfig::new_default(1);

    // Update matching and inflight
    {
        let mut pe = ProgressEntry::<UTConfig>::empty(20);
        pe.inflight = inflight_logs(5, 10);
        pe.new_updater(&engine_config).update_matching(Some(log_id(6)));
        assert_eq!(inflight_logs(6, 10), pe.inflight);
        assert_eq!(Some(&log_id(6)), pe.matching());
        assert_eq!(20, pe.searching_end);

        pe.new_updater(&engine_config).update_matching(Some(log_id(10)));
        assert_eq!(Inflight::None, pe.inflight);
        assert_eq!(Some(&log_id(10)), pe.matching());
        assert_eq!(20, pe.searching_end);
    }

    // `searching_end` should be updated
    {
        let mut pe = ProgressEntry::<UTConfig>::empty(20);
        pe.matching = Some(log_id(6));
        pe.inflight = inflight_logs(5, 20);

        pe.new_updater(&engine_config).update_matching(Some(log_id(20)));
        assert_eq!(21, pe.searching_end);
    }

    Ok(())
}

#[test]
fn test_update_conflicting() -> anyhow::Result<()> {
    let mut pe = ProgressEntry::<UTConfig>::empty(20);
    pe.matching = Some(log_id(3));
    pe.inflight = inflight_logs(5, 10);

    let engine_config = EngineConfig::new_default(1);
    pe.new_updater(&engine_config).update_conflicting(5, true);

    assert_eq!(Inflight::None, pe.inflight);
    assert_eq!(&Some(log_id(3)), pe.borrow());
    assert_eq!(5, pe.searching_end);

    Ok(())
}

/// LogStateReader impl for testing
struct LogState {
    last: Option<LogIdOf<UTConfig>>,
    snap_last: Option<LogIdOf<UTConfig>>,
    purge_upto: Option<LogIdOf<UTConfig>>,
    purged: Option<LogIdOf<UTConfig>>,
}

impl LogState {
    fn new(purge_upto: u64, snap_last: u64, last: u64) -> Self {
        Self {
            last: Some(log_id(last)),
            snap_last: Some(log_id(snap_last)),
            // `next_send()` only checks purge_upto, but not purged,
            // We just fake a purged
            purge_upto: Some(log_id(purge_upto)),
            purged: Some(log_id(purge_upto - 1)),
        }
    }
}

impl LogStateReader<UTConfig> for LogState {
    fn get_log_id(&self, index: u64) -> Option<LogIdOf<UTConfig>> {
        let x = Some(log_id(index));
        if x >= self.purged && x <= self.last { x } else { None }
    }

    fn ref_log_id(&self, _index: u64) -> Option<RefLogId<'_, UTConfig>> {
        unimplemented!("testing")
    }

    fn last_log_id(&self) -> Option<&LogIdOf<UTConfig>> {
        self.last.as_ref()
    }

    fn committed(&self) -> Option<&LogIdOf<UTConfig>> {
        unimplemented!("testing")
    }

    fn io_applied(&self) -> Option<&LogIdOf<UTConfig>> {
        todo!()
    }

    fn io_snapshot_last_log_id(&self) -> Option<&LogIdOf<UTConfig>> {
        todo!()
    }

    fn io_purged(&self) -> Option<&LogIdOf<UTConfig>> {
        todo!()
    }

    fn snapshot_last_log_id(&self) -> Option<&LogIdOf<UTConfig>> {
        self.snap_last.as_ref()
    }

    fn purge_upto(&self) -> Option<&LogIdOf<UTConfig>> {
        self.purge_upto.as_ref()
    }

    fn last_purged_log_id(&self) -> Option<&LogIdOf<UTConfig>> {
        self.purged.as_ref()
    }
}

#[test]
fn test_next_send() -> anyhow::Result<()> {
    // There is already inflight data, return it in an Error
    {
        let mut pe = ProgressEntry::<UTConfig>::empty(20);
        pe.inflight = inflight_logs(10, 11);
        let res = pe.next_send(&LogState::new(6, 10, 20), 100);
        assert_eq!(Err(&inflight_logs(10, 11)), res);
    }

    {
        //    matching,end
        //    4,5
        //    v
        // -----+------+-----+--->
        //      purged snap  last
        //      6      10    20

        let mut pe = ProgressEntry::<UTConfig>::empty(4);
        pe.matching = Some(log_id(4));

        let res = pe.next_send(&LogState::new(6, 10, 20), 100);
        assert_eq!(Ok(&Inflight::snapshot()), res);
    }
    {
        //    matching,end
        //    4 6
        //    v-v
        // -----+------+-----+--->
        //      purged snap  last
        //      6      10    20

        let mut pe = ProgressEntry::<UTConfig>::empty(6);
        pe.matching = Some(log_id(4));

        let res = pe.next_send(&LogState::new(6, 10, 20), 100);
        assert_eq!(Ok(&Inflight::snapshot()), res);
    }

    {
        //    matching,end
        //    4  7
        //    v--v
        // -----+------+-----+--->
        //      purged snap  last
        //      6      10    20

        let mut pe = ProgressEntry::<UTConfig>::empty(7);
        pe.matching = Some(log_id(4));

        let res = pe.next_send(&LogState::new(6, 10, 20), 100);
        assert_eq!(Ok(&inflight_logs(6, 20)), res);
    }

    {
        //    matching,end
        //    4              20
        //    v--------------v
        // -----+------+-----+--->
        //      purged snap  last
        //      6      10    20

        let mut pe = ProgressEntry::<UTConfig>::empty(20);
        pe.matching = Some(log_id(4));

        let res = pe.next_send(&LogState::new(6, 10, 20), 100);
        assert_eq!(Ok(&inflight_logs(6, 20)), res);
    }

    //-----------

    {
        //      matching,end
        //      6,7
        //      v
        // -----+------+-----+--->
        //      purged snap  last
        //      6      10    20

        let mut pe = ProgressEntry::<UTConfig>::empty(7);
        pe.matching = Some(log_id(6));

        let res = pe.next_send(&LogState::new(6, 10, 20), 100);
        assert_eq!(Ok(&inflight_logs(6, 20)), res);
    }

    {
        //      matching,end
        //      6, 8
        //      v--v
        // -----+------+-----+--->
        //      purged snap  last
        //      6      10    20

        let mut pe = ProgressEntry::<UTConfig>::empty(8);
        pe.matching = Some(log_id(6));

        let res = pe.next_send(&LogState::new(6, 10, 20), 100);
        assert_eq!(Ok(&inflight_logs(6, 20)), res);
    }

    {
        //      matching,end
        //      6,           20
        //      v------------v
        // -----+------+-----+--->
        //      purged snap  last
        //      6      10    20

        let mut pe = ProgressEntry::<UTConfig>::empty(20);
        pe.matching = Some(log_id(6));

        let res = pe.next_send(&LogState::new(6, 10, 20), 100);
        assert_eq!(Ok(&inflight_logs(6, 20)), res);
    }

    {
        //       matching,end
        //       7,          20
        //       v-----------v
        // -----+------+-----+--->
        //      purged snap  last
        //      6      10    20

        let mut pe = ProgressEntry::<UTConfig>::empty(20);
        pe.matching = Some(log_id(7));

        let res = pe.next_send(&LogState::new(6, 10, 20), 100);
        assert_eq!(Ok(&inflight_logs(7, 20)), res);
    }

    {
        //          matching,end
        //          7,8
        //          v
        // -----+------+-----+--->
        //      purged snap  last
        //      6      10    20

        let mut pe = ProgressEntry::<UTConfig>::empty(8);
        pe.matching = Some(log_id(7));

        let res = pe.next_send(&LogState::new(6, 10, 20), 100);
        assert_eq!(Ok(&inflight_logs(7, 20)), res);
    }

    {
        //                   matching,end
        //                   20,21
        //                   v
        // -----+------+-----+--->
        //      purged snap  last
        //      6      10    20

        let mut pe = ProgressEntry::<UTConfig>::empty(21);
        pe.matching = Some(log_id(20));

        let res = pe.next_send(&LogState::new(6, 10, 20), 100);
        assert_eq!(Err(&Inflight::None), res, "nothing to send");
    }

    // Test max_entries
    {
        //       matching,end
        //       7,          20
        //       v-----------v
        // -----+------+-----+--->
        //      purged snap  last
        //      6      10    20

        let mut pe = ProgressEntry::<UTConfig>::empty(20);
        pe.matching = Some(log_id(7));

        let res = pe.next_send(&LogState::new(6, 10, 20), 5);
        assert_eq!(Ok(&inflight_logs(7, 12)), res);
    }
    Ok(())
}
