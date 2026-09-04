use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::Membership;
use crate::MembershipState;
use crate::Vote;
use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::errors::ConflictingLogId;
use crate::progress::Inflight;
use crate::progress::entry::ProgressEntry;
use crate::progress::inflight_id::InflightId;
use crate::progress::stream_id::StreamId;
use crate::raft::ConflictHint;
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

fn conflict_at(expect: LogIdOf<UTConfig>) -> ConflictingLogId<UTConfig> {
    ConflictingLogId {
        expect,
        local: None,
        hint: None,
    }
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
            entry.data.inflight = Inflight::logs(Some(log_id(2, 1, 7)), Some(log_id(2, 1, 9)), first_id);
        });
    }

    tracing::info!(target = 2, inflight_id = 7, "apply the matching conflict response");
    {
        let mut expected = entry(&eng, 2);
        expected.data.inflight = Inflight::None;
        expected.data.searching_end = 7;

        eng.replication_handler().update_conflicting(2, conflict_at(log_id(2, 1, 7)), Some(first_id));

        let actual = entry(&eng, 2);
        assert_eq!(expected, actual);
    }

    tracing::info!(target = 2, inflight_id = 8, "replace the completed payload request");
    {
        update_entry(&mut eng, 2, |entry| {
            entry.data.searching_end = 10;
            entry.data.inflight = Inflight::logs(Some(log_id(2, 1, 3)), Some(log_id(2, 1, 9)), second_id);
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

        eng.replication_handler().update_conflicting(2, conflict_at(log_id(2, 1, 7)), Some(first_id));

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

        eng.replication_handler().update_conflicting(2, conflict_at(log_id(2, 1, 6)), None);

        let actual = entry(&eng, 2);
        assert_eq!(expected, actual);
    }

    tracing::info!(target = 99, "ignore a conflict for a removed target");
    {
        let expected_entries = entries(&eng);
        let expected_quorum = quorum_accepted(&eng);

        eng.replication_handler().update_conflicting(99, conflict_at(log_id(2, 1, 0)), Some(first_id));

        let actual_entries = entries(&eng);
        let actual_quorum = quorum_accepted(&eng);
        assert_eq!(expected_entries, actual_entries);
        assert_eq!(expected_quorum, actual_quorum);
    }

    Ok(())
}

#[test]
fn test_update_conflicting_uses_matching_tail_hint() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state.log_ids = LogIdList::new(None, [log_id(1, 1, 5), log_id(2, 1, 10)]);

    update_entry(&mut eng, 2, |entry| {
        *entry = ProgressEntry::empty(2, StreamId::new(2), 11)
            .with_inflight(Inflight::logs(Some(log_id(2, 1, 9)), Some(log_id(2, 1, 10)), InflightId::new(7)));
    });

    eng.replication_handler().update_conflicting(
        2,
        ConflictingLogId {
            expect: log_id(2, 1, 9),
            local: None,
            hint: Some(ConflictHint {
                last_log_id: Some(log_id(1, 1, 5)),
                committed_log_id: Some(log_id(1, 1, 3)),
            }),
        },
        Some(InflightId::new(7)),
    );

    let progress = entry(&eng, 2);
    assert_eq!(Some(&log_id(1, 1, 5)), progress.matching());
    assert_eq!(6, progress.data.searching_end);
    assert_eq!(Inflight::None, progress.data.inflight);

    Ok(())
}

#[test]
fn test_update_conflicting_uses_committed_hint_as_binary_search_lower_bound() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state.log_ids = LogIdList::new(None, [log_id(1, 1, 5), log_id(2, 1, 10)]);

    update_entry(&mut eng, 2, |entry| {
        *entry = ProgressEntry::empty(2, StreamId::new(2), 11)
            .with_inflight(Inflight::logs(Some(log_id(2, 1, 9)), Some(log_id(2, 1, 10)), InflightId::new(7)));
    });

    eng.replication_handler().update_conflicting(
        2,
        ConflictingLogId {
            expect: log_id(2, 1, 9),
            local: Some(log_id(3, 2, 9)),
            hint: Some(ConflictHint {
                last_log_id: Some(log_id(3, 2, 8)),
                committed_log_id: Some(log_id(1, 1, 5)),
            }),
        },
        Some(InflightId::new(7)),
    );

    let progress = entry(&eng, 2);
    assert_eq!(Some(&log_id(1, 1, 5)), progress.matching());
    assert_eq!(8, progress.data.searching_end);
    assert_eq!(Inflight::None, progress.data.inflight);

    Ok(())
}

#[test]
#[should_panic(expected = "follower committed log must exist on the leader")]
fn test_update_conflicting_asserts_committed_hint_matches_leader() {
    let mut eng = eng();
    eng.state.log_ids = LogIdList::new(None, [log_id(1, 1, 5), log_id(2, 1, 10)]);

    tracing::info!(target = 2, "reject a committed hint that violates Leader Completeness");
    {
        update_entry(&mut eng, 2, |entry| {
            *entry = ProgressEntry::empty(2, StreamId::new(2), 11).with_inflight(Inflight::logs(
                Some(log_id(2, 1, 9)),
                Some(log_id(2, 1, 10)),
                InflightId::new(7),
            ));
        });

        eng.replication_handler().update_conflicting(
            2,
            ConflictingLogId {
                expect: log_id(2, 1, 9),
                local: Some(log_id(3, 2, 9)),
                hint: Some(ConflictHint {
                    last_log_id: Some(log_id(3, 2, 8)),
                    committed_log_id: Some(log_id(3, 2, 5)),
                }),
            },
            Some(InflightId::new(7)),
        );
    }
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
            entry.data.inflight = Inflight::logs(Some(log_id(2, 1, 8)), Some(log_id(2, 1, 11)), first_id);
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

        eng.replication_handler().update_conflicting(1, conflict_at(log_id(2, 1, 8)), Some(first_id));

        let actual = entry(&eng, 1);
        assert_eq!(expected, actual);

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
            entry.data.inflight = Inflight::logs(Some(log_id(2, 1, 6)), Some(log_id(2, 1, 9)), third_id);
        });
        eng.replication_handler().allow_next_revert(3, true)?;

        let mut expected = entry(&eng, 3);
        expected.matching = None;
        expected.data.inflight = Inflight::None;
        expected.data.searching_end = 6;
        expected.data.allow_log_reversion = false;

        eng.replication_handler().update_conflicting(3, conflict_at(log_id(2, 1, 6)), Some(third_id));

        let actual = entry(&eng, 3);
        assert_eq!(expected, actual);

        let ordered = ordered_matching(&eng);
        let expected_ordered = vec![(2, Some(log_id(2, 1, 7))), (3, None), (1, None)];
        assert_eq!(expected_ordered, ordered);

        let expected_quorum = Some(log_id(2, 1, 7));
        let actual_quorum = quorum_accepted(&eng);
        assert_eq!(expected_quorum, actual_quorum);
    }

    Ok(())
}
