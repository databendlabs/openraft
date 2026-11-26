use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::Membership;
use crate::MembershipState;
use crate::Vote;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::progress::Inflight;
use crate::progress::inflight_id::InflightId;
use crate::replication::response::ReplicationResult;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::StoredMembershipOf;
use crate::utime::Leased;

fn m01() -> Membership<u64, ()> {
    Membership::<u64, ()>::new_with_defaults(vec![btreeset! {0,1}], [])
}

fn m123() -> Membership<u64, ()> {
    Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2,3}], [])
}

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(0);
    eng.state.enable_validation(false); // Disable validation for incomplete state

    eng.config.id = 2;
    eng.state.vote = Leased::new(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(2, 1),
    );
    eng.state.membership_state = MembershipState::new(
        Arc::new(StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 1)), m01())),
        Arc::new(StoredMembershipOf::<UTConfig>::new(Some(log_id(2, 1, 3)), m123())),
    );

    eng
}

#[test]
fn test_update_matching_no_leader() -> anyhow::Result<()> {
    // There is no leader, it should panic.

    let res = std::panic::catch_unwind(move || {
        let mut eng = eng();
        eng.replication_handler().update_matching(3, Some(log_id(1, 1, 2)), InflightId::new(1));
    });
    tracing::info!("res: {:?}", res);
    assert!(res.is_err());

    Ok(())
}

#[test]
fn test_update_matching() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.testing_new_leader();
    eng.output.take_commands();

    let mut rh = eng.replication_handler();
    // A log stream to each target: every acknowledgement of a stream carries its inflight id.
    let mut set_inflight = |id, prev| {
        let inflight = Inflight::logs_since(prev, InflightId::new(1));
        assert_eq!(
            Some(&inflight),
            rh.leader.progress.update_data_with(&id, |data| data.inflight = inflight).map(|data| &data.inflight)
        );
    };

    set_inflight(1, Some(log_id(2, 1, 3)));
    set_inflight(2, Some(log_id(1, 1, 0)));
    set_inflight(3, Some(log_id(1, 1, 1)));

    // progress: None, None, (1,2)
    {
        rh.update_matching(3, Some(log_id(1, 1, 2)), InflightId::new(1));
        assert_eq!(None, rh.state.local_committed());
        assert_eq!(0, rh.output.take_commands().len());
    }

    // progress: None, (2,1), (1,2); quorum-ed: (1,2), not at leader vote, not committed
    {
        rh.output.clear_commands();
        rh.update_matching(2, Some(log_id(2, 1, 1)), InflightId::new(1));
        assert_eq!(None, rh.state.local_committed());
        assert_eq!(0, rh.output.take_commands().len());
    }

    // progress: None, (2,1), (2,3); committed: (2,1)
    {
        rh.output.clear_commands();
        rh.update_matching(3, Some(log_id(2, 1, 3)), InflightId::new(1));
        assert_eq!(Some(&log_id(2, 1, 1)), rh.state.local_committed());
        assert_eq!(
            vec![Command::ReplicateCommitted {
                committed: Some(log_id(2, 1, 1))
            },],
            rh.output.take_commands()
        );
        assert_eq!(Some(&log_id(2, 1, 1)), rh.state.io_state.apply_progress.accepted());
        assert_eq!(None, rh.state.io_state.apply_progress.submitted());
    }

    // progress: (2,4), (2,1), (2,3); committed: (1,3)
    {
        rh.output.clear_commands();
        rh.update_matching(1, Some(log_id(2, 1, 4)), InflightId::new(1));
        assert_eq!(Some(&log_id(2, 1, 3)), rh.state.local_committed());
        assert_eq!(
            vec![Command::ReplicateCommitted {
                committed: Some(log_id(2, 1, 3))
            },],
            rh.output.take_commands()
        );
        assert_eq!(Some(&log_id(2, 1, 3)), rh.state.io_state.apply_progress.accepted());
        assert_eq!(None, rh.state.io_state.apply_progress.submitted());
    }

    Ok(())
}

#[test]
fn test_update_matching_relaxed_read_bounds_committed_by_applied() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.config.enable_relaxed_read = true;
    eng.testing_new_leader();
    eng.output.take_commands();

    let mut rh = eng.replication_handler();
    for (id, prev) in [(1, log_id(2, 1, 3)), (2, log_id(1, 1, 0)), (3, log_id(1, 1, 1))] {
        let inflight = Inflight::probe(Some(prev), Some(log_id(2, 1, 4)), InflightId::new(1));
        rh.leader.progress.update_data_with(&id, |data| data.inflight = inflight);
    }

    // committed: (2,1); the leader has applied nothing, so followers may apply nothing.
    {
        rh.update_matching(2, Some(log_id(2, 1, 1)), InflightId::new(1));
        rh.update_matching(3, Some(log_id(2, 1, 3)), InflightId::new(1));
        assert_eq!(Some(&log_id(2, 1, 1)), rh.state.cluster_committed());
        assert_eq!(
            vec![Command::ReplicateCommitted { committed: None }],
            rh.output.take_commands()
        );
    }

    // committed: (2,3); the leader has applied (2,1), which bounds what followers may apply.
    {
        rh.state.io_state.apply_progress.submit(log_id(2, 1, 1));
        rh.state.io_state.apply_progress.flush(log_id(2, 1, 1));
        rh.update_matching(1, Some(log_id(2, 1, 4)), InflightId::new(1));
        assert_eq!(Some(&log_id(2, 1, 3)), rh.state.cluster_committed());
        assert_eq!(
            vec![Command::ReplicateCommitted {
                committed: Some(log_id(2, 1, 1))
            }],
            rh.output.take_commands()
        );
    }

    Ok(())
}

#[test]
fn test_empty_ack_clears_inflight_without_updating_matching() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.testing_new_leader();
    eng.output.take_commands();

    let target = 1;
    let inflight_id = InflightId::new(1);
    let inflight = Inflight::probe(None, Some(log_id(2, 1, 4)), inflight_id);

    let mut rh = eng.replication_handler();
    let data = rh.leader.progress.update_data_with(&target, |data| data.inflight = inflight);
    let stream_id = data.unwrap().stream_id;

    let progress = crate::replication::response::Progress {
        target,
        result: Ok(ReplicationResult(Ok(None))),
        hint: None,
    };
    rh.update_progress(stream_id, progress, Some(inflight_id));

    let entry = rh.leader.progress.try_get(&target).unwrap();
    assert_eq!(None, entry.matching());
    assert_eq!(Inflight::None, entry.data.inflight);

    Ok(())
}

/// A success without an inflight id, such as the answer to an update of the commit index sent on
/// its own, changes no progress, whatever the leader tracks for the target: no plan, a clear, or a
/// probe that replaced a dropped plan. The target may have lost its log since it answered. Here a
/// heartbeat's conflict at log 3 has reset the progress of node 3 under allowed log reversion, and
/// node 1 matches log 3, so counting the answer would let nodes 1 and 3 accept log 3 as a quorum.
#[test]
fn test_untracked_success_changes_no_progress() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.testing_new_leader();
    eng.output.take_commands();

    let mut rh = eng.replication_handler();
    rh.leader.progress.update_entry_with(&1, |entry| {
        entry.matching = Some(log_id(2, 1, 3));
        entry.data.searching_end = 4;
    });
    let data = rh.leader.progress.update_data_with(&3, |data| data.searching_end = 3);
    let stream_id = data.unwrap().stream_id;

    let success = crate::replication::response::Progress {
        target: 3,
        result: Ok(ReplicationResult(Ok(Some(log_id(2, 1, 3))))),
        hint: None,
    };
    let clear = Inflight::Clear {
        prev: Some(log_id(2, 1, 3)),
        inflight_id: InflightId::new(5),
    };
    let probe = Inflight::probe(None, Some(log_id(2, 1, 3)), InflightId::new(6));

    for inflight in [Inflight::None, clear, probe] {
        tracing::info!(%inflight, "node 3 answers log 3 without an inflight id: nothing changes");
        {
            rh.leader.progress.update_data_with(&3, |data| data.inflight = inflight);
            let expected = rh.leader.progress.iter().cloned().collect::<Vec<_>>();

            rh.update_progress(stream_id, success.clone(), None);

            let actual = rh.leader.progress.iter().cloned().collect::<Vec<_>>();
            assert_eq!(expected, actual);
            assert_eq!(&None, rh.leader.progress.quorum_accepted());
            assert_eq!(Vec::<Command<UTConfig>>::new(), rh.output.take_commands());
        }
    }

    tracing::info!(
        inflight_id = 6,
        "the same answer to the probe advances node 3 and commits log 3"
    );
    {
        rh.update_progress(stream_id, success, Some(InflightId::new(6)));

        let entry = rh.leader.progress.try_get(&3).unwrap();
        assert_eq!(Some(&log_id(2, 1, 3)), entry.matching());
        assert_eq!(Inflight::None, entry.data.inflight);
        assert_eq!(
            vec![Command::ReplicateCommitted {
                committed: Some(log_id(2, 1, 3))
            }],
            rh.output.take_commands()
        );
    }

    Ok(())
}
