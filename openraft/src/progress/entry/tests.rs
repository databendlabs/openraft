use crate::LogId;
use crate::RaftState;
use crate::engine::EngineConfig;
use crate::engine::LogIdList;
use crate::engine::testing::UTConfig;
use crate::progress::entry::ProgressEntry;
use crate::progress::inflight::Inflight;
use crate::progress::inflight_id::InflightId;
use crate::progress::stream_id::StreamId;
use crate::type_config::alias::LeaderIdOf;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::SnapshotMetaOf;
use crate::type_config::alias::StoredMembershipOf;
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
    let mut pe = ProgressEntry::<UTConfig>::empty(0, StreamId::new(0), 20);
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
        let mut pe = ProgressEntry::<UTConfig>::empty(0, StreamId::new(0), 20);
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
        let mut pe = ProgressEntry::<UTConfig>::empty(0, StreamId::new(0), 20);
        pe.matching = Some(log_id(6));
        pe.inflight = inflight_logs(5, 20);

        pe.new_updater(&engine_config).update_matching(Some(log_id(20)), Some(InflightId::new(0)));
        assert_eq!(21, pe.searching_end);
    }

    Ok(())
}

#[test]
fn test_update_conflicting() -> anyhow::Result<()> {
    let mut pe = ProgressEntry::<UTConfig>::empty(0, StreamId::new(0), 20);
    pe.matching = Some(log_id(3));
    pe.inflight = inflight_logs(5, 10);

    let engine_config = EngineConfig::new_default(1);
    pe.new_updater(&engine_config).update_conflicting(5, Some(InflightId::new(0)));

    assert_eq!(Inflight::None, pe.inflight);
    assert_eq!(Some(&log_id(3)), pe.matching());
    assert_eq!(5, pe.searching_end);

    Ok(())
}

fn new_raft_state(purge_upto: u64, snap_last: u64, last: u64) -> RaftState<UTConfig> {
    let mut st = RaftState::new(0);

    // TODO: setup log_ids

    st.purge_upto = Some(log_id(purge_upto));
    st.snapshot_meta = SnapshotMetaOf::<UTConfig> {
        last_log_id: Some(log_id(snap_last)),
        last_membership: StoredMembershipOf::<UTConfig>::default(),
        snapshot_id: "".to_string(),
    };

    st.log_ids = LogIdList::new(None, [log_id(purge_upto - 1), log_id(last)]);

    st
}

#[test]
fn test_next_send_inflight_busy() -> anyhow::Result<()> {
    // There is already inflight data, return it in an Error
    let mut pe = ProgressEntry::<UTConfig>::empty(0, StreamId::new(0), 20);
    pe.inflight = inflight_logs(10, 11);

    let res = pe.next_send(&mut new_raft_state(6, 10, 20), 100);
    assert_eq!(Err(&inflight_logs(10, 11)), res);

    Ok(())
}

#[test]
fn test_next_send_snapshot_matching_purged() -> anyhow::Result<()> {
    //    matching,end
    //    4,5
    //    v
    // -----+------+-----+--->
    //      purged snap  last
    //      6      10    20
    //
    // Pipeline regime (matching.next_index() == searching_end): the matching point is
    // known to be 4, but the logs the follower needs next (5..=6) are purged.
    // Snapshot condition 1.
    let mut pe = ProgressEntry::<UTConfig>::empty(0, StreamId::new(0), 5);
    pe.matching = Some(log_id(4));

    let res = pe.next_send(&mut new_raft_state(6, 10, 20), 100);
    assert_eq!(Ok(&Inflight::snapshot(InflightId::new(1))), res);

    Ok(())
}

#[test]
fn test_next_send_snapshot_search_range_purged() -> anyhow::Result<()> {
    //    matching,end
    //    4 6
    //    v-v
    // -----+------+-----+--->
    //      purged snap  last
    //      6      10    20
    //
    // Probing regime: every candidate matching position (4, 5) lies strictly below
    // the purge boundary (6). Snapshot condition 1.
    let mut pe = ProgressEntry::<UTConfig>::empty(0, StreamId::new(0), 6);
    pe.matching = Some(log_id(4));

    let res = pe.next_send(&mut new_raft_state(6, 10, 20), 100);
    assert_eq!(Ok(&Inflight::snapshot(InflightId::new(1))), res);

    Ok(())
}

#[test]
fn test_next_send_probe_at_purge_boundary() -> anyhow::Result<()> {
    //    matching,end
    //    4  7
    //    v--v
    // -----+------+-----+--->
    //      purged snap  last
    //      6      10    20
    //
    // searching_end == purge_upto_next: the purge boundary (6) is itself still a
    // candidate matching position, and its log id is known as the snapshot's last
    // log id. Probe with prev at the boundary instead of falling back to a snapshot.
    let mut pe = ProgressEntry::<UTConfig>::empty(0, StreamId::new(0), 7);
    pe.matching = Some(log_id(4));

    let res = pe.next_send(&mut new_raft_state(6, 10, 20), 100);
    assert_eq!(Ok(&inflight_logs(6, 20).with_id(1)), res);

    Ok(())
}

#[test]
fn test_next_send_probe_clamped_to_purge_boundary() -> anyhow::Result<()> {
    //    matching,end
    //    4              20
    //    v--------------v
    // -----+------+-----+--->
    //      purged snap  last
    //      6      10    20
    //
    // The binary-search midpoint, calc_mid(5, 20) = 5, is below the purge boundary;
    // the probe start is clamped up to the boundary: prev = log id 6.
    let mut pe = ProgressEntry::<UTConfig>::empty(0, StreamId::new(0), 20);
    pe.matching = Some(log_id(4));

    let res = pe.next_send(&mut new_raft_state(6, 10, 20), 100);
    assert_eq!(Ok(&inflight_logs(6, 20).with_id(1)), res);

    Ok(())
}

#[test]
fn test_next_send_pipeline_at_purge_boundary() -> anyhow::Result<()> {
    //      matching,end
    //      6,7
    //      v
    // -----+------+-----+--->
    //      purged snap  last
    //      6      10    20
    //
    // matching.next_index() == searching_end, enter pipeline mode
    // (the matching point is exactly the purge boundary: nothing needed is purged)
    let mut pe = ProgressEntry::<UTConfig>::empty(0, StreamId::new(0), 7);
    pe.matching = Some(log_id(6));

    let res = pe.next_send(&mut new_raft_state(6, 10, 20), 100);
    assert_eq!(
        Ok(&Inflight::logs_since(Some(log_id(6)), InflightId::new(1))),
        res,
        "pipeline mode: matching.next == searching_end"
    );

    Ok(())
}

#[test]
fn test_next_send_probe_from_matching_next_short_range() -> anyhow::Result<()> {
    //      matching,end
    //      6, 8
    //      v--v
    // -----+------+-----+--->
    //      purged snap  last
    //      6      10    20
    //
    // Probing: the search range [7, 8) is too narrow for a non-zero binary-search
    // offset, so the probe starts right after matching: prev = log id 6.
    let mut pe = ProgressEntry::<UTConfig>::empty(0, StreamId::new(0), 8);
    pe.matching = Some(log_id(6));

    let res = pe.next_send(&mut new_raft_state(6, 10, 20), 100);
    assert_eq!(Ok(&inflight_logs(6, 20).with_id(1)), res);

    Ok(())
}

#[test]
fn test_next_send_probe_from_matching_next_wide_range() -> anyhow::Result<()> {
    //      matching,end
    //      6,           20
    //      v------------v
    // -----+------+-----+--->
    //      purged snap  last
    //      6      10    20
    //
    // Probing: calc_mid(7, 20) = 7 (offset (20-7)/16*8 = 0), so the probe still
    // starts right after matching: prev = log id 6.
    let mut pe = ProgressEntry::<UTConfig>::empty(0, StreamId::new(0), 20);
    pe.matching = Some(log_id(6));

    let res = pe.next_send(&mut new_raft_state(6, 10, 20), 100);
    assert_eq!(Ok(&inflight_logs(6, 20).with_id(1)), res);

    Ok(())
}

#[test]
fn test_next_send_probe_from_matching_next_above_boundary() -> anyhow::Result<()> {
    //       matching,end
    //       7,          20
    //       v-----------v
    // -----+------+-----+--->
    //      purged snap  last
    //      6      10    20
    //
    // Probing with matching above the purge boundary: no clamping is involved;
    // the probe starts right after matching: prev = log id 7.
    let mut pe = ProgressEntry::<UTConfig>::empty(0, StreamId::new(0), 20);
    pe.matching = Some(log_id(7));

    let res = pe.next_send(&mut new_raft_state(6, 10, 20), 100);
    assert_eq!(Ok(&inflight_logs(7, 20).with_id(1)), res);

    Ok(())
}

#[test]
fn test_next_send_pipeline_above_purge_boundary() -> anyhow::Result<()> {
    //          matching,end
    //          7,8
    //          v
    // -----+------+-----+--->
    //      purged snap  last
    //      6      10    20
    //
    // matching.next_index() == searching_end, enter pipeline mode
    let mut pe = ProgressEntry::<UTConfig>::empty(0, StreamId::new(0), 8);
    pe.matching = Some(log_id(7));

    let res = pe.next_send(&mut new_raft_state(6, 10, 20), 100);
    assert_eq!(
        Ok(&Inflight::logs_since(Some(log_id(7)), InflightId::new(1))),
        res,
        "pipeline mode: matching.next == searching_end"
    );

    Ok(())
}

#[test]
fn test_next_send_pipeline_caught_up() -> anyhow::Result<()> {
    //                   matching,end
    //                   20,21
    //                   v
    // -----+------+-----+--->
    //      purged snap  last
    //      6      10    20
    //
    // matching.next_index() == searching_end, enter pipeline mode
    // (even though follower is fully caught up: an empty open-ended stream is valid)
    let mut pe = ProgressEntry::<UTConfig>::empty(0, StreamId::new(0), 21);
    pe.matching = Some(log_id(20));

    let res = pe.next_send(&mut new_raft_state(6, 10, 20), 100);
    assert_eq!(
        Ok(&Inflight::logs_since(Some(log_id(20)), InflightId::new(1))),
        res,
        "pipeline mode: fully caught up"
    );

    Ok(())
}

#[test]
fn test_next_send_probe_capped_by_max_entries() -> anyhow::Result<()> {
    //       matching,end
    //       7,          20
    //       v-----------v
    // -----+------+-----+--->
    //      purged snap  last
    //      6      10    20
    //
    // max_entries = 5 caps the probe range: entries 8..=12 are sent.
    let mut pe = ProgressEntry::<UTConfig>::empty(0, StreamId::new(0), 20);
    pe.matching = Some(log_id(7));

    let res = pe.next_send(&mut new_raft_state(6, 10, 20), 5);
    assert_eq!(Ok(&inflight_logs(7, 12).with_id(1)), res);

    Ok(())
}

#[test]
fn test_next_send_fully_purged_unknown_follower() -> anyhow::Result<()> {
    //                     end
    //                     21
    //                     v
    // ----------------+--->
    //     purged/snap/last
    //                20
    //
    // matching = None; probing, but the leader log is fully purged
    // (purge_upto == last_log == 20): there is no entry for a probe to carry.
    // Snapshot condition 2.
    let mut pe = ProgressEntry::<UTConfig>::empty(0, StreamId::new(0), 21);

    let res = pe.next_send(&mut new_raft_state(20, 20, 20), 100);
    assert_eq!(Ok(&Inflight::snapshot(InflightId::new(1))), res);

    Ok(())
}

#[test]
fn test_next_send_not_pipeline_when_gap_exists() -> anyhow::Result<()> {
    //       matching  searching_end
    //       7         20
    //       v---------v
    // -----+------+-----+--->
    //      purged snap  last
    //      6      10    20
    //
    // NOT pipeline mode: matching.next_index() != searching_end — the exact matching
    // point is not determined yet, so this is a probing Logs, not a LogsSince stream.
    let mut pe = ProgressEntry::<UTConfig>::empty(0, StreamId::new(0), 20);
    pe.matching = Some(log_id(7));

    let res = pe.next_send(&mut new_raft_state(6, 10, 20), 100);
    assert_eq!(Ok(&inflight_logs(7, 20).with_id(1)), res, "not pipeline: gap exists");

    Ok(())
}

#[test]
fn test_next_send_not_pipeline_without_matching() -> anyhow::Result<()> {
    //  matching=None  searching_end
    //                 20
    //                 v
    // -----+------+-----+--->
    //      purged snap  last
    //      6      10    20
    //
    // NOT pipeline mode: no matching yet.
    // calc_mid(0, 20) = 0 + 20/16*8 = 8, so the probe starts at 8: prev = log id 7.
    let mut pe = ProgressEntry::<UTConfig>::empty(0, StreamId::new(0), 20);

    let res = pe.next_send(&mut new_raft_state(6, 10, 20), 100);
    assert_eq!(Ok(&inflight_logs(7, 20).with_id(1)), res, "not pipeline: no matching");

    Ok(())
}

#[test]
fn test_next_send_probe_from_mid_of_search_range() -> anyhow::Result<()> {
    //    matching,end
    //    4               21
    //    v---------------v
    // -----+------+-----+--->
    //      purged snap  last
    //      6      10    20
    //
    // The search range [5, 21) is wide enough for a non-zero binary-search offset:
    // calc_mid(5, 21) = 5 + 16/16*8 = 13. The probe starts from the midpoint,
    // not right after matching: prev = log id 12.
    let mut pe = ProgressEntry::<UTConfig>::empty(0, StreamId::new(0), 21);
    pe.matching = Some(log_id(4));

    let res = pe.next_send(&mut new_raft_state(6, 10, 20), 100);
    assert_eq!(Ok(&inflight_logs(12, 20).with_id(1)), res);

    Ok(())
}

#[test]
fn test_next_send_snapshot_fully_purged_matching_behind() -> anyhow::Result<()> {
    //          matching   end
    //          15         21
    //          v----------v
    // ----------------+--->
    //     purged/snap/last
    //                20
    //
    // Fully purged leader with a known lower bound (matching = 15): still probing
    // (matching.next_index() < searching_end) and still no entry to probe with.
    // Snapshot condition 2.
    let mut pe = ProgressEntry::<UTConfig>::empty(0, StreamId::new(0), 21);
    pe.matching = Some(log_id(15));

    let res = pe.next_send(&mut new_raft_state(20, 20, 20), 100);
    assert_eq!(Ok(&Inflight::snapshot(InflightId::new(1))), res);

    Ok(())
}

#[test]
fn test_next_send_pipeline_fully_purged_caught_up() -> anyhow::Result<()> {
    //             matching,end
    //             20,21
    //             v
    // ----------------+--->
    //     purged/snap/last
    //                20
    //
    // Fully purged leader with a fully caught-up follower: the matching point is
    // known, so this is pipeline, not probing — snapshot condition 2 must not fire.
    // An empty open-ended stream is valid.
    let mut pe = ProgressEntry::<UTConfig>::empty(0, StreamId::new(0), 21);
    pe.matching = Some(log_id(20));

    let res = pe.next_send(&mut new_raft_state(20, 20, 20), 100);
    assert_eq!(Ok(&Inflight::logs_since(Some(log_id(20)), InflightId::new(1))), res);

    Ok(())
}
