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
use crate::progress::Inflight;
use crate::progress::entry::ProgressEntry;
use crate::progress::inflight_id::InflightId;
use crate::progress::stream_id::StreamId;
use crate::replication::response::Progress;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::StoredMembershipOf;
use crate::utime::Leased;

fn m23() -> Membership<u64, ()> {
    Membership::new_with_defaults(vec![btreeset! {2, 3}], [])
}

/// Leader 2 replicates to node 3, whose operation in flight is `inflight`, and the logs up to
/// `purge_upto` wait to be purged.
fn eng(inflight: Inflight<UTConfig>, purge_upto: Option<LogIdOf<UTConfig>>) -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(2);
    eng.state.enable_validation(false); // The fixture can start with a purge that waits.
    eng.config.id = 2;
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

    eng.state.purge_upto = purge_upto;
    eng.state.last_inflight_id = 10;
    eng.leader.as_mut().unwrap().progress.update_data_with(&3, |data| data.inflight = inflight);
    eng
}

fn entry_of_3(eng: &Engine<UTConfig>) -> ProgressEntry<UTConfig> {
    eng.leader.as_ref().unwrap().progress.try_get(&3).unwrap().clone()
}

fn error() -> Progress<UTConfig> {
    Progress {
        target: 3,
        result: Err("connection reset".to_string()),
        hint: None,
    }
}

/// An RPC error resets only the operation it belongs to. An error with the ID of an earlier
/// operation, without an ID, or from a replaced stream leaves node 3's progress entry as it is,
/// and a log plan in flight keeps holding back the purge of the logs it reads.
#[test]
fn test_update_progress_ignores_an_error_of_another_operation() {
    let current = InflightId::new(5);
    let probe = Inflight::probe(Some(log_id(1, 1, 4)), Some(log_id(2, 2, 6)), current);
    let logs_since = Inflight::logs_since(Some(log_id(1, 1, 4)), current);
    let cases = [
        (probe, Some(log_id(2, 2, 5))),
        (logs_since, Some(log_id(2, 2, 5))),
        // A snapshot reads no log, so no purge waits for it.
        (Inflight::snapshot(current), None),
        (Inflight::None, None),
    ];

    for (inflight, purge_upto) in cases {
        let mut eng = eng(inflight, purge_upto);
        let stream_id = entry_of_3(&eng).data.stream_id;
        let before = entry_of_3(&eng);

        tracing::info!(%inflight, "--- errors that are not of the operation in flight change nothing");
        {
            let stale = [
                (stream_id, Some(InflightId::new(4))),
                (stream_id, None),
                (StreamId::new(999), Some(current)),
            ];
            for (stream_id, inflight_id) in stale {
                eng.replication_handler().update_progress(stream_id, error(), inflight_id);

                assert_eq!(before, entry_of_3(&eng));
                assert_eq!(Vec::<Command<UTConfig>>::new(), eng.output.take_commands());
            }
        }
    }
}

/// An RPC error with the ID of the operation in flight resets it, so that it is retried: a log plan
/// becomes a clear that waits for the replication task, and a snapshot ends at once.
#[test]
fn test_update_progress_resets_the_operation_of_the_error() {
    let current = InflightId::new(5);
    let probe = Inflight::probe(Some(log_id(1, 1, 4)), Some(log_id(2, 2, 6)), current);
    let logs_since = Inflight::logs_since(Some(log_id(1, 1, 4)), current);

    for inflight in [probe, logs_since] {
        let mut eng = eng(inflight, Some(log_id(2, 2, 5)));
        let stream_id = entry_of_3(&eng).data.stream_id;

        tracing::info!(%inflight, "--- the error of the plan in flight drops the plan");
        {
            eng.replication_handler().update_progress(stream_id, error(), Some(current));

            let clear = InflightId::new(11);
            let expected = vec![Command::ClearReplicate {
                target: 3,
                inflight_id: clear,
            }];
            assert_eq!(expected, eng.output.take_commands());
            let expected = Inflight::Clear {
                prev: Some(log_id(1, 1, 4)),
                inflight_id: clear,
            };
            assert_eq!(expected, entry_of_3(&eng).data.inflight);
        }
    }

    tracing::info!("--- the error of the snapshot in flight ends it");
    {
        let mut eng = eng(Inflight::snapshot(current), Some(log_id(2, 2, 5)));
        let stream_id = entry_of_3(&eng).data.stream_id;

        eng.replication_handler().update_progress(stream_id, error(), Some(current));

        assert_eq!(Inflight::None, entry_of_3(&eng).data.inflight);
        assert_eq!(
            vec![Command::PurgeLog { upto: log_id(2, 2, 5) }],
            eng.output.take_commands()
        );
    }
}
