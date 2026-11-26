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
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::log_id_range::LogIdRange;
use crate::progress::Inflight;
use crate::progress::entry::ProgressEntry;
use crate::progress::inflight_id::InflightId;
use crate::progress::stream_id::StreamId;
use crate::raft::ConflictHint;
use crate::replication::replicate::Replicate;
use crate::replication::response::Progress;
use crate::replication::response::ReplicationResult;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::StoredMembershipOf;
use crate::utime::Leased;

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(1);
    eng.state.enable_validation(false);
    eng.state.vote = Leased::new(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(2, 1),
    );
    let membership = Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1, 2, 3}], []);
    let committed = StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 1)), membership.clone());
    let effective = StoredMembershipOf::<UTConfig>::new(Some(log_id(2, 1, 3)), membership);
    eng.state.membership_state = MembershipState::new(Arc::new(committed), Arc::new(effective));
    eng.testing_new_leader();
    eng.output.clear_commands();
    eng
}

fn update_entry<F>(eng: &mut Engine<UTConfig>, target: u64, update: F)
where F: FnOnce(&mut ProgressEntry<UTConfig>) {
    let leader = eng.leader.as_mut().unwrap();
    let result = leader.progress.update_entry_with(&target, update);
    let found = result.is_some();
    assert!(found);
}

fn entries(eng: &Engine<UTConfig>) -> Vec<ProgressEntry<UTConfig>> {
    let leader = eng.leader.as_ref().unwrap();
    leader.progress.iter().cloned().collect()
}

fn entry(eng: &Engine<UTConfig>, target: u64) -> ProgressEntry<UTConfig> {
    let leader = eng.leader.as_ref().unwrap();
    let entry = leader.progress.try_get(&target).unwrap();
    entry.clone()
}

fn ordered_matching(eng: &Engine<UTConfig>) -> Vec<(u64, Option<LogIdOf<UTConfig>>)> {
    let leader = eng.leader.as_ref().unwrap();
    leader.progress.collect_mapped(|entry| (entry.id, entry.matching))
}

fn quorum_accepted(eng: &Engine<UTConfig>) -> Option<LogIdOf<UTConfig>> {
    let leader = eng.leader.as_ref().unwrap();
    *leader.progress.quorum_accepted()
}

#[test]
fn test_update_conflicting_response_identity() -> anyhow::Result<()> {
    let mut eng = eng();
    let first_id = InflightId::new(7);
    let second_id = InflightId::new(8);

    tracing::info!(target = 2, "install the first payload request");
    {
        update_entry(&mut eng, 2, |entry| {
            entry.matching = Some(log_id(2, 1, 3));
            entry.data.searching_end = 10;
            entry.data.inflight = Inflight::probe(Some(log_id(2, 1, 7)), Some(log_id(2, 1, 9)), first_id);
        });
    }

    tracing::info!(target = 2, inflight_id = 7, "apply the matching conflict response");
    {
        let mut expected = entry(&eng, 2);
        expected.data.inflight = Inflight::None;
        expected.data.searching_end = 7;

        let applied = eng.replication_handler().update_conflicting(2, log_id(2, 1, 7), Some(first_id));

        assert!(applied);
        let actual = entry(&eng, 2);
        assert_eq!(expected, actual);
    }

    tracing::info!(target = 2, inflight_id = 8, "replace the completed payload request");
    {
        update_entry(&mut eng, 2, |entry| {
            entry.data.searching_end = 10;
            entry.data.inflight = Inflight::probe(Some(log_id(2, 1, 3)), Some(log_id(2, 1, 9)), second_id);
        });
    }

    tracing::info!(
        target = 2,
        inflight_id = 7,
        "ignore a stale conflict from the first request"
    );
    {
        let expected_entry = entry(&eng, 2);
        let expected_entries = entries(&eng);
        let expected_quorum = quorum_accepted(&eng);

        let applied = eng.replication_handler().update_conflicting(2, log_id(2, 1, 7), Some(first_id));

        assert!(!applied);
        let actual_entry = entry(&eng, 2);
        let actual_entries = entries(&eng);
        let actual_quorum = quorum_accepted(&eng);
        assert_eq!(expected_entry, actual_entry);
        assert_eq!(expected_entries, actual_entries);
        assert_eq!(expected_quorum, actual_quorum);
    }

    tracing::info!(target = 2, "apply a heartbeat conflict without clearing payload state");
    {
        let mut expected = entry(&eng, 2);
        expected.data.searching_end = 6;

        let applied = eng.replication_handler().update_conflicting(2, log_id(2, 1, 6), None);

        assert!(applied);
        let actual = entry(&eng, 2);
        assert_eq!(expected, actual);
    }

    tracing::info!(target = 99, "ignore a conflict for a removed target");
    {
        let expected_entries = entries(&eng);
        let expected_quorum = quorum_accepted(&eng);

        let applied = eng.replication_handler().update_conflicting(99, log_id(2, 1, 0), Some(first_id));

        assert!(!applied);
        let actual_entries = entries(&eng);
        let actual_quorum = quorum_accepted(&eng);
        assert_eq!(expected_entries, actual_entries);
        assert_eq!(expected_quorum, actual_quorum);
    }

    Ok(())
}

#[test]
fn test_update_conflicting_reversion() -> anyhow::Result<()> {
    let mut eng = eng();
    let first_id = InflightId::new(1);
    let third_id = InflightId::new(3);

    tracing::info!("establish ordered voter progress and a quorum watermark");
    {
        update_entry(&mut eng, 1, |entry| {
            entry.matching = Some(log_id(2, 1, 8));
            entry.data.searching_end = 12;
            entry.data.inflight = Inflight::probe(Some(log_id(2, 1, 8)), Some(log_id(2, 1, 11)), first_id);
        });
        update_entry(&mut eng, 2, |entry| {
            entry.matching = Some(log_id(2, 1, 7));
            entry.data.searching_end = 12;
        });
        update_entry(&mut eng, 3, |entry| {
            entry.matching = Some(log_id(2, 1, 6));
            entry.data.searching_end = 12;
        });

        let ordered = ordered_matching(&eng);
        let expected_ordered = vec![
            (1, Some(log_id(2, 1, 8))),
            (2, Some(log_id(2, 1, 7))),
            (3, Some(log_id(2, 1, 6))),
        ];
        assert_eq!(expected_ordered, ordered);

        let expected_quorum = Some(log_id(2, 1, 7));
        let actual_quorum = quorum_accepted(&eng);
        assert_eq!(expected_quorum, actual_quorum);
    }

    tracing::info!(target = 1, conflict = 8, "apply globally enabled log reversion");
    {
        eng.config.allow_log_reversion = true;
        let mut expected = entry(&eng, 1);
        expected.matching = None;
        expected.data.inflight = Inflight::None;
        expected.data.searching_end = 8;

        eng.replication_handler().update_conflicting(1, log_id(2, 1, 8), Some(first_id));

        let actual = entry(&eng, 1);
        assert_eq!(expected, actual);

        // The conflict answers the probe, whose only request the stream has sent: no clear.
        let commands = eng.output.take_commands();
        assert_eq!(Vec::<Command<UTConfig>>::new(), commands);

        let ordered = ordered_matching(&eng);
        let expected_ordered = vec![(2, Some(log_id(2, 1, 7))), (3, Some(log_id(2, 1, 6))), (1, None)];
        assert_eq!(expected_ordered, ordered);

        let expected_quorum = Some(log_id(2, 1, 7));
        let actual_quorum = quorum_accepted(&eng);
        assert_eq!(expected_quorum, actual_quorum);
    }

    tracing::info!(target = 3, conflict = 6, "consume one-shot log reversion permission");
    {
        eng.config.allow_log_reversion = false;
        update_entry(&mut eng, 3, |entry| {
            entry.data.searching_end = 10;
            entry.data.inflight = Inflight::probe(Some(log_id(2, 1, 6)), Some(log_id(2, 1, 9)), third_id);
        });
        eng.replication_handler().allow_next_revert(3, true)?;

        let mut expected = entry(&eng, 3);
        expected.matching = None;
        expected.data.inflight = Inflight::None;
        expected.data.searching_end = 6;
        expected.data.allow_log_reversion = false;

        eng.replication_handler().update_conflicting(3, log_id(2, 1, 6), Some(third_id));

        let actual = entry(&eng, 3);
        assert_eq!(expected, actual);

        let commands = eng.output.take_commands();
        assert_eq!(Vec::<Command<UTConfig>>::new(), commands);

        let ordered = ordered_matching(&eng);
        let expected_ordered = vec![(2, Some(log_id(2, 1, 7))), (3, None), (1, None)];
        assert_eq!(expected_ordered, ordered);

        let expected_quorum = Some(log_id(2, 1, 7));
        let actual_quorum = quorum_accepted(&eng);
        assert_eq!(expected_quorum, actual_quorum);
    }

    Ok(())
}

/// A heartbeat's conflict that finds the target's log reverted drops its log plan, a `LogsSince`
/// plan or a probe, which the target's stream never hears of. The leader asks the replication task
/// to confirm that it no longer reads the log for the plan, and starts no other plan and purges the
/// entries the plan held back only once the task answers.
#[test]
fn reverted_log_purges_only_after_the_stream_confirms_the_clear() {
    let logs_since = Inflight::logs_since(Some(log_id(2, 1, 5)), InflightId::new(7));
    let probe = Inflight::probe(Some(log_id(2, 1, 5)), Some(log_id(2, 1, 9)), InflightId::new(7));

    for (plan, searching_end) in [(logs_since, 6), (probe, 10)] {
        let mut eng = eng();
        eng.config.allow_log_reversion = true;
        eng.state.log_ids = LogIdList::new(None, [log_id(1, 1, 1), log_id(2, 1, 3), log_id(2, 1, 10)]);
        eng.state.purge_upto = Some(log_id(2, 1, 8));
        eng.state.last_inflight_id = 10;
        update_entry(&mut eng, 2, |entry| {
            entry.matching = Some(log_id(2, 1, 5));
            entry.data.searching_end = searching_end;
            entry.data.inflight = plan;
        });
        let stream_id = entry(&eng, 2).data.stream_id;
        let clear = InflightId::new(11);

        tracing::info!(target = 2, %plan, "a heartbeat at index 5 finds the log of node 2 reverted");
        {
            let response = Progress {
                target: 2,
                result: Ok(ReplicationResult(Err(log_id(2, 1, 5)))),
                hint: None,
            };
            eng.replication_handler().update_progress(stream_id, response, None);

            let expected = vec![Command::ClearReplicate {
                target: 2,
                inflight_id: clear,
            }];
            assert_eq!(expected, eng.output.take_commands());
            let expected = Inflight::Clear {
                prev: Some(log_id(2, 1, 5)),
                inflight_id: clear,
            };
            assert_eq!(expected, entry(&eng, 2).data.inflight);
        }

        tracing::info!(target = 2, "node 2 gets no new plan before the clear is confirmed");
        {
            eng.replication_handler().initiate_replication();

            let commands = eng.output.take_commands();
            let to_node_2 = commands.iter().any(|cmd| match cmd {
                Command::Replicate { target, .. } | Command::ReplicateSnapshot { target, .. } => *target == 2,
                _ => false,
            });
            assert!(!to_node_2, "expect no plan for node 2, got {:?}", commands);
        }

        tracing::info!(target = 2, "the replication task confirms the clear: the purge runs");
        {
            eng.replication_handler().update_cleared(stream_id, 2, clear);

            let expected = vec![Command::PurgeLog { upto: log_id(2, 1, 8) }];
            assert_eq!(expected, eng.output.take_commands());
            assert_eq!(Inflight::None, entry(&eng, 2).data.inflight);
        }
    }
}

/// A heartbeat's conflict that finds the log of node 2 reverted drops its log plan, whether log
/// reversion is allowed for every target or once for node 2. An answer that node 2 sent before it
/// lost its log carries the dropped plan's inflight id: it neither raises the reset progress nor
/// completes the probe that replaces the plan, while the probe's own answer does.
#[test]
fn reverted_log_ignores_the_answers_of_a_dropped_log_plan() -> anyhow::Result<()> {
    let dropped = InflightId::new(7);
    let probe = Inflight::probe(Some(log_id(2, 1, 5)), Some(log_id(2, 1, 9)), dropped);
    let logs_since = Inflight::logs_since(Some(log_id(2, 1, 5)), dropped);
    let stream_id = StreamId::new(2);
    let clear = InflightId::new(11);
    let new_probe = InflightId::new(12);
    let stale_answer = Progress {
        target: 2,
        result: Ok(ReplicationResult(Ok(Some(log_id(2, 1, 9))))),
        hint: None,
    };

    for allow_once in [false, true] {
        for (plan, searching_end) in [(probe, 10), (logs_since, 6)] {
            let mut eng = eng();
            eng.state.log_ids = LogIdList::new(None, [log_id(1, 1, 2), log_id(2, 1, 10)]);
            eng.state.last_inflight_id = 10;
            update_entry(&mut eng, 2, |entry| {
                *entry = ProgressEntry::empty(2, stream_id, searching_end).with_inflight(plan);
                entry.matching = Some(log_id(2, 1, 5));
            });
            // Node 3 is busy, so that only node 2 gets a new plan.
            update_entry(&mut eng, 3, |entry| {
                entry.data.inflight = Inflight::snapshot(InflightId::new(1))
            });
            if allow_once {
                eng.replication_handler().allow_next_revert(2, true)?;
            } else {
                eng.config.allow_log_reversion = true;
            }

            tracing::info!(%plan, allow_once, "a heartbeat at index 5 finds the log of node 2 reverted");
            {
                let response = Progress {
                    target: 2,
                    result: Ok(ReplicationResult(Err(log_id(2, 1, 5)))),
                    hint: None,
                };
                eng.replication_handler().update_progress(stream_id, response, None);

                let inflight = Inflight::Clear {
                    prev: Some(log_id(2, 1, 5)),
                    inflight_id: clear,
                };
                let expected = ProgressEntry::empty(2, stream_id, 5).with_inflight(inflight);
                assert_eq!(expected, entry(&eng, 2));
                let expected = vec![Command::ClearReplicate {
                    target: 2,
                    inflight_id: clear,
                }];
                assert_eq!(expected, eng.output.take_commands());
            }

            tracing::info!(inflight_id = 7, "an answer of the dropped plan changes nothing");
            {
                let expected = entry(&eng, 2);

                eng.replication_handler().update_progress(stream_id, stale_answer.clone(), Some(dropped));

                assert_eq!(expected, entry(&eng, 2));
                assert_eq!(Vec::<Command<UTConfig>>::new(), eng.output.take_commands());
            }

            tracing::info!("the replication task confirms the clear, and node 2 gets a new probe");
            {
                eng.replication_handler().update_cleared(stream_id, 2, clear);
                eng.replication_handler().initiate_replication();

                let log_id_range = LogIdRange::new(None, Some(log_id(2, 1, 7)));
                let expected = vec![Command::Replicate {
                    target: 2,
                    req: Replicate::new_probe(log_id_range, new_probe),
                }];
                assert_eq!(expected, eng.output.take_commands());
            }

            tracing::info!(
                inflight_id = 7,
                "an answer of the dropped plan does not complete the new probe"
            );
            {
                let expected = entry(&eng, 2);

                eng.replication_handler().update_progress(stream_id, stale_answer.clone(), Some(dropped));

                assert_eq!(expected, entry(&eng, 2));
                assert_eq!(Vec::<Command<UTConfig>>::new(), eng.output.take_commands());
            }

            tracing::info!(inflight_id = 12, "the answer of the new probe advances matching");
            {
                let response = Progress {
                    target: 2,
                    result: Ok(ReplicationResult(Ok(Some(log_id(2, 1, 7))))),
                    hint: None,
                };
                eng.replication_handler().update_progress(stream_id, response, Some(new_probe));

                let mut expected = ProgressEntry::empty(2, stream_id, 8);
                expected.matching = Some(log_id(2, 1, 7));
                assert_eq!(expected, entry(&eng, 2));
                assert_eq!(Vec::<Command<UTConfig>>::new(), eng.output.take_commands());
            }
        }
    }

    Ok(())
}

/// A heartbeat's conflict that finds the log of node 2 reverted drops its snapshot transfer, which
/// reads no log, so no clear is needed. The leader's next plan for node 2 is another snapshot,
/// because the search range still lies below the purged logs. An answer of the dropped transfer
/// neither raises the reset progress nor completes the new transfer, while the new one's does.
#[test]
fn reverted_log_ignores_the_answer_of_a_dropped_snapshot() -> anyhow::Result<()> {
    let dropped = InflightId::new(7);
    let new_snapshot = InflightId::new(11);
    let stream_id = StreamId::new(2);
    let snapshot_answer = Progress {
        target: 2,
        result: Ok(ReplicationResult(Ok(Some(log_id(2, 1, 8))))),
        hint: None,
    };

    for allow_once in [false, true] {
        let mut eng = eng();
        eng.state.log_ids = LogIdList::new(Some(log_id(2, 1, 8)), [log_id(2, 1, 10)]);
        eng.state.purge_upto = Some(log_id(2, 1, 8));
        eng.state.last_inflight_id = 10;
        update_entry(&mut eng, 2, |entry| {
            *entry = ProgressEntry::empty(2, stream_id, 6).with_inflight(Inflight::snapshot(dropped));
            entry.matching = Some(log_id(2, 1, 5));
        });
        // Node 3 is busy, so that only node 2 gets a new plan.
        update_entry(&mut eng, 3, |entry| {
            entry.data.inflight = Inflight::snapshot(InflightId::new(1))
        });
        if allow_once {
            eng.replication_handler().allow_next_revert(2, true)?;
        } else {
            eng.config.allow_log_reversion = true;
        }

        tracing::info!(allow_once, "a heartbeat at index 5 finds the log of node 2 reverted");
        {
            let response = Progress {
                target: 2,
                result: Ok(ReplicationResult(Err(log_id(2, 1, 5)))),
                hint: None,
            };
            eng.replication_handler().update_progress(stream_id, response, None);

            let expected = ProgressEntry::empty(2, stream_id, 5);
            assert_eq!(expected, entry(&eng, 2));
            assert_eq!(Vec::<Command<UTConfig>>::new(), eng.output.take_commands());
        }

        tracing::info!(inflight_id = 7, "the answer of the dropped transfer changes nothing");
        {
            let expected = entry(&eng, 2);

            eng.replication_handler().update_progress(stream_id, snapshot_answer.clone(), Some(dropped));

            assert_eq!(expected, entry(&eng, 2));
            assert_eq!(Vec::<Command<UTConfig>>::new(), eng.output.take_commands());
        }

        tracing::info!("node 2 gets a new snapshot transfer");
        {
            eng.replication_handler().initiate_replication();

            let leader_vote = eng.leader.as_ref().unwrap().committed_vote.clone();
            let expected = vec![Command::ReplicateSnapshot {
                leader_vote,
                target: 2,
                inflight_id: new_snapshot,
            }];
            assert_eq!(expected, eng.output.take_commands());
        }

        tracing::info!(
            inflight_id = 7,
            "the answer of the dropped transfer does not complete the new one"
        );
        {
            let expected = entry(&eng, 2);

            eng.replication_handler().update_progress(stream_id, snapshot_answer.clone(), Some(dropped));

            assert_eq!(expected, entry(&eng, 2));
            assert_eq!(Vec::<Command<UTConfig>>::new(), eng.output.take_commands());
        }

        tracing::info!(inflight_id = 11, "the answer of the new transfer advances matching");
        {
            eng.replication_handler().update_progress(stream_id, snapshot_answer.clone(), Some(new_snapshot));

            let mut expected = ProgressEntry::empty(2, stream_id, 9);
            expected.matching = Some(log_id(2, 1, 8));
            assert_eq!(expected, entry(&eng, 2));
            assert_eq!(Vec::<Command<UTConfig>>::new(), eng.output.take_commands());
        }
    }

    Ok(())
}

/// A heartbeat's conflict that finds the log of node 2 reverted keeps the clear that waits for its
/// replication task: no follower answers a clear, and the logs that the dropped plan's stream may
/// still read stay unpurged until the task confirms it.
#[test]
fn reverted_log_keeps_a_pending_clear() {
    let mut eng = eng();
    eng.config.allow_log_reversion = true;
    eng.state.log_ids = LogIdList::new(None, [log_id(1, 1, 2), log_id(2, 1, 10)]);
    eng.state.purge_upto = Some(log_id(2, 1, 8));
    let stream_id = StreamId::new(2);
    let clear = Inflight::Clear {
        prev: Some(log_id(2, 1, 5)),
        inflight_id: InflightId::new(9),
    };
    update_entry(&mut eng, 2, |entry| {
        *entry = ProgressEntry::empty(2, stream_id, 6).with_inflight(clear);
        entry.matching = Some(log_id(2, 1, 5));
    });

    tracing::info!("a heartbeat at index 5 resets the progress of node 2 and keeps its clear");
    {
        let response = Progress {
            target: 2,
            result: Ok(ReplicationResult(Err(log_id(2, 1, 5)))),
            hint: None,
        };
        eng.replication_handler().update_progress(stream_id, response, None);

        let expected = ProgressEntry::empty(2, stream_id, 5).with_inflight(clear);
        assert_eq!(expected, entry(&eng, 2));
        assert_eq!(Vec::<Command<UTConfig>>::new(), eng.output.take_commands());
    }

    tracing::info!(inflight_id = 7, "an answer of the dropped plan changes nothing");
    {
        let expected = entry(&eng, 2);
        let response = Progress {
            target: 2,
            result: Ok(ReplicationResult(Ok(Some(log_id(2, 1, 9))))),
            hint: None,
        };

        eng.replication_handler().update_progress(stream_id, response, Some(InflightId::new(7)));

        assert_eq!(expected, entry(&eng, 2));
        assert_eq!(Vec::<Command<UTConfig>>::new(), eng.output.take_commands());
    }

    tracing::info!(
        inflight_id = 9,
        "the replication task confirms the clear: the purge runs"
    );
    {
        eng.replication_handler().update_cleared(stream_id, 2, InflightId::new(9));

        let expected = vec![Command::PurgeLog { upto: log_id(2, 1, 8) }];
        assert_eq!(expected, eng.output.take_commands());
        assert_eq!(Inflight::None, entry(&eng, 2).data.inflight);
    }
}

fn hint_probe_engine() -> Engine<UTConfig> {
    let mut eng = eng();
    eng.state.log_ids = LogIdList::new(None, [log_id(1, 1, 2), log_id(2, 1, 10)]);
    update_entry(&mut eng, 1, |entry| {
        entry.matching = Some(log_id(2, 1, 10));
        entry.data.searching_end = 11;
    });
    update_entry(&mut eng, 2, |entry| {
        *entry = ProgressEntry::empty(2, StreamId::new(2), 11);
        entry.data.inflight = Inflight::probe(Some(log_id(2, 1, 9)), Some(log_id(2, 1, 10)), InflightId::new(7));
    });
    eng
}

fn hinted_conflict() -> Progress<UTConfig> {
    let hint = ConflictHint::new(vec![log_id(2, 1, 5)]);
    Progress {
        target: 2,
        result: Ok(ReplicationResult(Err(log_id(2, 1, 9)))),
        hint: Some(hint),
    }
}

#[test]
fn hinted_prefix_waits_for_durable_acknowledgement() {
    let mut eng = hint_probe_engine();
    let response = hinted_conflict();
    let stream_id = StreamId::new(2);
    eng.replication_handler().update_progress(stream_id, response, Some(InflightId::new(7)));
    let expected = ProgressEntry::empty(2, stream_id, 6).with_probe_start(6);
    let progress = entry(&eng, 2);
    assert_eq!(expected, progress);
    let quorum = quorum_accepted(&eng);
    assert_eq!(None, quorum);

    eng.replication_handler().initiate_replication();
    eng.replication_handler().update_matching(2, Some(log_id(2, 1, 10)), InflightId::new(1));
    let quorum = quorum_accepted(&eng);
    let expected = Some(log_id(2, 1, 10));
    assert_eq!(expected, quorum);
}

#[test]
fn hinted_first_sample_waits_for_durable_acknowledgement() {
    let mut eng = hint_probe_engine();
    let mut response = hinted_conflict();
    response.hint = Some(ConflictHint::new(vec![log_id(1, 1, 2), log_id(1, 2, 5)]));
    let stream_id = StreamId::new(2);
    eng.replication_handler().update_progress(stream_id, response, Some(InflightId::new(7)));
    let expected = ProgressEntry::empty(2, stream_id, 5).with_probe_start(3);
    let progress = entry(&eng, 2);
    assert_eq!(expected, progress);
    let quorum = quorum_accepted(&eng);
    assert_eq!(None, quorum);
}

#[test]
fn hint_without_a_shared_log_id_only_narrows_the_search_range() {
    let mut eng = hint_probe_engine();
    let mut response = hinted_conflict();
    response.hint = Some(ConflictHint::new(vec![log_id(1, 2, 5)]));
    let stream_id = StreamId::new(2);

    eng.replication_handler().update_progress(stream_id, response, Some(InflightId::new(7)));

    let expected = ProgressEntry::empty(2, stream_id, 5);
    let progress = entry(&eng, 2);
    assert_eq!(expected, progress);
}

#[test]
fn hint_bounds_do_not_expand_an_existing_search_range() {
    let mut eng = hint_probe_engine();
    update_entry(&mut eng, 2, |entry| entry.data.searching_end = 4);
    let response = hinted_conflict();
    let stream_id = StreamId::new(2);
    eng.replication_handler().update_progress(stream_id, response, Some(InflightId::new(7)));
    let expected = ProgressEntry::empty(2, stream_id, 4);
    let progress = entry(&eng, 2);
    assert_eq!(expected, progress);
}

#[test]
fn stale_hinted_responses_leave_all_progress_unchanged() {
    let stream_id = StreamId::new(2);
    let other_stream = StreamId::new(99);
    for (stream, request) in [(stream_id, 8), (other_stream, 7)] {
        let mut eng = hint_probe_engine();
        let expected = entries(&eng);
        let response = hinted_conflict();
        eng.replication_handler().update_progress(stream, response, Some(InflightId::new(request)));
        let actual = entries(&eng);
        assert_eq!(expected, actual);
    }
}

/// A conflict without an inflight id, such as a heartbeat's, cannot be tied to the current
/// request, so its hint is ignored.
#[test]
fn hint_of_a_conflict_without_inflight_id_is_ignored() {
    let mut eng = hint_probe_engine();
    let response = Progress {
        target: 2,
        result: Ok(ReplicationResult(Err(log_id(2, 1, 10)))),
        hint: Some(ConflictHint::new(vec![log_id(2, 1, 9)])),
    };
    let stream_id = StreamId::new(2);

    eng.replication_handler().update_progress(stream_id, response, None);

    let inflight = Inflight::probe(Some(log_id(2, 1, 9)), Some(log_id(2, 1, 10)), InflightId::new(7));
    let expected = ProgressEntry::empty(2, stream_id, 10).with_inflight(inflight);
    let progress = entry(&eng, 2);
    assert_eq!(expected, progress);
}

#[test]
fn loss_of_unflushed_hinted_prefix_restarts_search() {
    let mut eng = hint_probe_engine();
    let stream_id = StreamId::new(2);
    eng.replication_handler().update_progress(stream_id, hinted_conflict(), Some(InflightId::new(7)));
    eng.replication_handler().initiate_replication();
    eng.replication_handler().update_conflicting(2, log_id(2, 1, 5), Some(InflightId::new(1)));
    let expected = ProgressEntry::empty(2, stream_id, 5);
    let progress = entry(&eng, 2);
    assert_eq!(expected, progress);
    let quorum = quorum_accepted(&eng);
    assert_eq!(None, quorum);
}

#[test]
fn empty_hint_cannot_lower_a_durable_matching_prefix() {
    let mut eng = hint_probe_engine();
    update_entry(&mut eng, 2, |entry| entry.matching = Some(log_id(2, 1, 3)));
    let mut response = hinted_conflict();
    response.hint = Some(ConflictHint::new(vec![]));
    let stream_id = StreamId::new(2);
    eng.replication_handler().update_progress(stream_id, response, Some(InflightId::new(7)));
    let progress = entry(&eng, 2);
    let matching = progress.matching().cloned();
    let expected = Some(log_id(2, 1, 3));
    assert_eq!(expected, matching);
    assert_eq!(4, progress.data.searching_end);
    assert_eq!(None, progress.data.probe_start);
}
