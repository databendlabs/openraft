use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::Membership;
use crate::MembershipState;
use crate::Vote;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::engine::TargetProgress;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::progress::Inflight;
use crate::progress::entry::ProgressEntry;
use crate::progress::inflight_id::InflightId;
use crate::progress::stream_id::StreamId;
use crate::raft_state::LogStateReader;
use crate::replication::response::Progress;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::StoredMembershipOf;
use crate::utime::Leased;
use crate::vote::raft_vote::RaftVoteExt;

fn m23() -> Membership<u64, ()> {
    Membership::new_with_defaults(vec![btreeset! {2, 3}], [])
}

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(2);
    eng.state.enable_validation(false); // The fixture starts with no pending purge.
    eng.config.id = 2;
    // The leader is newer than every entry in its log.
    eng.state.vote = Leased::new(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(3, 2),
    );
    eng.state.membership_state = MembershipState::new(
        Arc::new(StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 2)), m23())),
        Arc::new(StoredMembershipOf::<UTConfig>::new(Some(log_id(2, 2, 3)), m23())),
    );
    eng.state.log_ids = LogIdList::new(Some(log_id(1, 1, 2)), [log_id(1, 1, 4), log_id(2, 2, 6)]);
    eng.state.server_state = eng.calc_server_state();
    eng.testing_new_leader();
    eng.output.clear_commands();
    eng
}

#[test]
fn test_try_purge_log_already_purged() {
    let mut eng = eng();
    eng.state.purge_upto = Some(log_id(1, 1, 2));

    eng.replication_handler().try_purge_log();

    assert_eq!(
        (
            Some(log_id(1, 1, 2)),
            Some(log_id(1, 1, 2)),
            vec![log_id(1, 1, 4), log_id(2, 2, 6)],
            vec![],
        ),
        (
            eng.state.last_purged_log_id().cloned(),
            eng.state.purge_upto().cloned(),
            eng.state.log_ids.key_log_ids().to_vec(),
            eng.output.take_commands(),
        )
    );
}

#[test]
fn test_try_purge_log_postpones_when_logs_are_inflight() {
    let mut eng = eng();
    eng.state.purge_upto = Some(log_id(2, 2, 5));
    eng.leader.as_mut().unwrap().progress.update_data_with(&3, |data| {
        data.inflight = Inflight::probe(Some(log_id(1, 1, 4)), Some(log_id(2, 2, 6)), InflightId::new(1));
    });

    eng.replication_handler().try_purge_log();

    assert_eq!(
        (
            Some(log_id(1, 1, 2)),
            Some(log_id(2, 2, 5)),
            vec![log_id(1, 1, 4), log_id(2, 2, 6)],
            vec![],
        ),
        (
            eng.state.last_purged_log_id().cloned(),
            eng.state.purge_upto().cloned(),
            eng.state.log_ids.key_log_ids().to_vec(),
            eng.output.take_commands(),
        )
    );
}

#[test]
fn test_try_purge_log_retries_after_inflight_clears() {
    let mut eng = eng();
    eng.state.purge_upto = Some(log_id(2, 2, 5));
    eng.leader.as_mut().unwrap().progress.update_data_with(&3, |data| {
        data.inflight = Inflight::probe(Some(log_id(1, 1, 4)), Some(log_id(2, 2, 6)), InflightId::new(1));
    });

    eng.replication_handler().try_purge_log();
    eng.leader.as_mut().unwrap().progress.update_data_with(&3, |data| {
        data.inflight = Inflight::None;
    });
    eng.replication_handler().try_purge_log();

    assert_eq!(
        (
            Some(log_id(2, 2, 5)),
            Some(log_id(2, 2, 5)),
            vec![log_id(2, 2, 6)],
            vec![Command::PurgeLog { upto: log_id(2, 2, 5) }],
        ),
        (
            eng.state.last_purged_log_id().cloned(),
            eng.state.purge_upto().cloned(),
            eng.state.log_ids.key_log_ids().to_vec(),
            eng.output.take_commands(),
        )
    );
}

#[test]
fn test_try_purge_log_excludes_inflight_previous_log() {
    let mut eng = eng();
    eng.state.purge_upto = Some(log_id(2, 2, 5));
    eng.leader.as_mut().unwrap().progress.update_data_with(&3, |data| {
        data.inflight = Inflight::probe(Some(log_id(2, 2, 5)), Some(log_id(2, 2, 6)), InflightId::new(1));
    });

    eng.replication_handler().try_purge_log();

    assert_eq!(
        (
            Some(log_id(2, 2, 5)),
            Some(log_id(2, 2, 5)),
            vec![log_id(2, 2, 6)],
            vec![Command::PurgeLog { upto: log_id(2, 2, 5) }],
        ),
        (
            eng.state.last_purged_log_id().cloned(),
            eng.state.purge_upto().cloned(),
            eng.state.log_ids.key_log_ids().to_vec(),
            eng.output.take_commands(),
        )
    );
}

/// An RPC error drops the target's log plan, but the purge that the plan held back waits: a
/// transport that pulls requests on a task of its own may still read the log for the plan. The
/// leader asks the replication task to confirm the clear, and purges only once the task answers it.
/// Another error of the dropped plan, a stale answer, or an answer from a replaced stream does not
/// end the wait.
#[test]
fn test_try_purge_log_after_rpc_error_waits_for_the_clear() {
    let logs_since = Inflight::logs_since(Some(log_id(1, 1, 4)), InflightId::new(1));
    let probe = Inflight::probe(Some(log_id(1, 1, 4)), Some(log_id(2, 2, 6)), InflightId::new(1));
    let error = Progress {
        target: 3,
        result: Err("connection reset".to_string()),
        hint: None,
    };
    let clear = InflightId::new(11);
    let inflight_of_3 = |eng: &Engine<UTConfig>| {
        let leader = eng.leader.as_ref().unwrap();
        leader.progress.try_get(&3).unwrap().data.inflight
    };

    for inflight in [logs_since, probe] {
        let mut eng = eng();
        eng.state.purge_upto = Some(log_id(2, 2, 5));
        eng.state.last_inflight_id = 10;
        eng.leader.as_mut().unwrap().progress.update_data_with(&3, |data| data.inflight = inflight);
        let stream_id = eng.leader.as_ref().unwrap().progress.try_get(&3).unwrap().data.stream_id;

        tracing::info!(%inflight, "--- an RPC error answers the request of node 3");
        {
            eng.replication_handler().update_progress(stream_id, error.clone(), Some(InflightId::new(1)));

            let expected = vec![Command::ClearReplicate {
                target: 3,
                inflight_id: clear,
            }];
            assert_eq!(expected, eng.output.take_commands());
            let expected = Inflight::Clear {
                prev: Some(log_id(1, 1, 4)),
                inflight_id: clear,
            };
            assert_eq!(expected, inflight_of_3(&eng));
        }

        tracing::info!("--- nothing but the answer to the clear lets the purge run");
        {
            eng.replication_handler().update_progress(stream_id, error.clone(), Some(InflightId::new(1)));
            eng.replication_handler().update_cleared(stream_id, 3, InflightId::new(10));
            eng.replication_handler().update_cleared(StreamId::new(999), 3, clear);

            assert_eq!(Vec::<Command<UTConfig>>::new(), eng.output.take_commands());
            assert_eq!(Some(log_id(1, 1, 2)), eng.state.last_purged_log_id().cloned());
        }

        tracing::info!("--- the replication task answers the clear: the purge runs");
        {
            eng.replication_handler().update_cleared(stream_id, 3, clear);

            assert_eq!(
                vec![Command::PurgeLog { upto: log_id(2, 2, 5) }],
                eng.output.take_commands()
            );
            assert_eq!(Inflight::None, inflight_of_3(&eng));
        }
    }

    tracing::info!("--- an RPC error of a snapshot, which reads no log, purges at once");
    {
        let mut eng = eng();
        eng.state.purge_upto = Some(log_id(2, 2, 5));
        let snapshot = Inflight::snapshot(InflightId::new(1));
        eng.leader.as_mut().unwrap().progress.update_data_with(&3, |data| data.inflight = snapshot);
        let stream_id = eng.leader.as_ref().unwrap().progress.try_get(&3).unwrap().data.stream_id;

        eng.replication_handler().update_progress(stream_id, error, Some(InflightId::new(1)));

        assert_eq!(
            vec![Command::PurgeLog { upto: log_id(2, 2, 5) }],
            eng.output.take_commands()
        );
        assert_eq!(Inflight::None, inflight_of_3(&eng));
    }
}

/// Leader 2 of voters 2 and 3, with learner 4, whose logs up to (2, 2, 5) wait to be purged.
fn eng_with_learner_4() -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(2);
    eng.state.enable_validation(false); // The fixture starts with a purge that waits.
    eng.config.id = 2;
    eng.state.vote = Leased::new(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(3, 2),
    );
    let m23_4 = Membership::new_with_defaults(vec![btreeset! {2, 3}], btreeset! {4});
    eng.state.membership_state = MembershipState::new(
        Arc::new(StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 2)), m23())),
        Arc::new(StoredMembershipOf::<UTConfig>::new(Some(log_id(2, 2, 3)), m23_4)),
    );
    eng.state.log_ids = LogIdList::new(Some(log_id(1, 1, 2)), [log_id(1, 1, 4), log_id(2, 2, 6)]);
    eng.state.server_state = eng.calc_server_state();
    eng.testing_new_leader();
    eng.output.clear_commands();
    eng.state.purge_upto = Some(log_id(2, 2, 5));
    eng
}

/// Removing learner 4, whose stream reads the logs that wait to be purged, lets the purge run
/// right after the streams are rebuilt, with no answer from a stream. A stream of a retained
/// target that reads those logs still holds it back.
#[test]
fn test_append_membership_retries_the_pending_purge() {
    let streaming = Inflight::logs_since(Some(log_id(1, 1, 4)), InflightId::new(1));
    let entry_of =
        |eng: &Engine<UTConfig>, target: u64| eng.leader.as_ref().unwrap().progress.try_get(&target).unwrap().clone();
    let rebuild = |progress: ProgressEntry<UTConfig>| Command::RebuildReplicationStreams {
        leader_vote: Vote::new_committed(3, 2).to_committed(),
        targets: vec![TargetProgress {
            target: 3,
            target_node: (),
            progress,
        }],
        close_old_streams: false,
    };

    tracing::info!("--- learner 4 alone holds the purge back: removing it purges at once");
    {
        let mut eng = eng_with_learner_4();
        eng.leader.as_mut().unwrap().progress.update_data_with(&4, |data| data.inflight = streaming);
        eng.replication_handler().try_purge_log();
        assert_eq!(Vec::<Command<UTConfig>>::new(), eng.output.take_commands());

        eng.replication_handler().append_membership(&log_id(3, 2, 7), &m23());

        let expected = vec![rebuild(entry_of(&eng, 3)), Command::PurgeLog { upto: log_id(2, 2, 5) }];
        assert_eq!(expected, eng.output.take_commands());
        assert_eq!(Some(&log_id(2, 2, 5)), eng.state.last_purged_log_id());
    }

    tracing::info!("--- node 3 also reads the logs: removing learner 4 does not purge");
    {
        let mut eng = eng_with_learner_4();
        for target in [3, 4] {
            eng.leader.as_mut().unwrap().progress.update_data_with(&target, |data| data.inflight = streaming);
        }

        eng.replication_handler().append_membership(&log_id(3, 2, 7), &m23());

        assert_eq!(vec![rebuild(entry_of(&eng, 3))], eng.output.take_commands());
        assert_eq!(Some(&log_id(1, 1, 2)), eng.state.last_purged_log_id());
    }

    tracing::info!("--- no purge is pending, or it is already accepted: nothing more is emitted");
    {
        for purge_upto in [None, Some(log_id(1, 1, 2))] {
            let mut eng = eng_with_learner_4();
            eng.state.purge_upto = purge_upto;

            eng.replication_handler().append_membership(&log_id(3, 2, 7), &m23());

            assert_eq!(vec![rebuild(entry_of(&eng, 3))], eng.output.take_commands());
            assert_eq!(Some(&log_id(1, 1, 2)), eng.state.last_purged_log_id());
        }
    }
}
