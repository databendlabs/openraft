use std::io::Cursor;
use std::time::Duration;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::Membership;
use crate::StoredMembership;
use crate::Vote;
use crate::core::sm;
use crate::engine::Command;
use crate::engine::Condition;
use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::engine::Respond;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::raft::SnapshotResponse;
use crate::raft_state::io_state::log_io_id::LogIOId;
use crate::storage::Snapshot;
use crate::storage::SnapshotMeta;
use crate::type_config::TypeConfigExt;
use crate::vote::raft_vote::RaftVoteExt;

fn m12() -> Membership<UTConfig> {
    Membership::<UTConfig>::new_with_defaults(vec![btreeset! {1,2}], [])
}

fn m1234() -> Membership<UTConfig> {
    Membership::<UTConfig>::new_with_defaults(vec![btreeset! {1,2,3,4}], [])
}

fn eng() -> Engine<UTConfig> {
    let mut eng: Engine<UTConfig> = Engine::testing_default(0);
    eng.state.enable_validation(false); // Disable validation for incomplete state

    eng.state.vote.update(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(2, 1),
    );
    eng.state.apply_progress_mut().accept(log_id(4, 1, 5));
    eng.state.log_ids = LogIdList::new(vec![
        //
        log_id(2, 1, 2),
        log_id(3, 1, 5),
        log_id(4, 1, 6),
        log_id(4, 1, 8),
    ]);
    eng.state.snapshot_meta = SnapshotMeta {
        last_log_id: Some(log_id(2, 1, 2)),
        last_membership: StoredMembership::new(Some(log_id(1, 1, 1)), m12()),
        snapshot_id: "1-2-3-4".to_string(),
    };
    eng.state.server_state = eng.calc_server_state();

    eng
}

#[test]
fn test_handle_install_full_snapshot_lt_last_snapshot() -> anyhow::Result<()> {
    // Snapshot will not be installed because new `last_log_id` is less or equal current
    // `snapshot_meta.last_log_id`.
    //
    // It should respond at once.

    let mut eng = eng();

    let curr_vote = *eng.state.vote_ref();

    let (tx, _rx) = UTConfig::<()>::oneshot();

    eng.handle_install_full_snapshot(
        curr_vote,
        Snapshot {
            meta: SnapshotMeta {
                last_log_id: Some(log_id(1, 1, 2)),
                last_membership: StoredMembership::new(Some(log_id(1, 1, 1)), m1234()),
                snapshot_id: "1-2-3-4".to_string(),
            },
            snapshot: Cursor::new(vec![0u8]),
        },
        tx,
    );

    assert_eq!(
        SnapshotMeta {
            last_log_id: Some(log_id(2, 1, 2)),
            last_membership: StoredMembership::new(Some(log_id(1, 1, 1)), m12()),
            snapshot_id: "1-2-3-4".to_string(),
        },
        eng.state.snapshot_meta
    );

    let (dummy_tx, _rx) = UTConfig::<()>::oneshot();
    assert_eq!(
        vec![
            //
            Command::Respond {
                when: None,
                resp: Respond::new(SnapshotResponse::new(curr_vote), dummy_tx),
            },
        ],
        eng.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_handle_install_full_snapshot_no_conflict() -> anyhow::Result<()> {
    // Snapshot will be installed and there are no conflicting logs.
    // The response should be sent after the snapshot is installed.

    let mut eng = eng();

    let curr_vote = *eng.state.vote_ref();

    let (tx, _rx) = UTConfig::<()>::oneshot();

    eng.handle_install_full_snapshot(
        curr_vote,
        Snapshot {
            meta: SnapshotMeta {
                last_log_id: Some(log_id(4, 1, 6)),
                last_membership: StoredMembership::new(Some(log_id(1, 1, 1)), m1234()),
                snapshot_id: "1-2-3-4".to_string(),
            },
            snapshot: Cursor::new(vec![0u8]),
        },
        tx,
    );

    assert_eq!(
        SnapshotMeta {
            last_log_id: Some(log_id(4, 1, 6)),
            last_membership: StoredMembership::new(Some(log_id(1, 1, 1)), m1234()),
            snapshot_id: "1-2-3-4".to_string(),
        },
        eng.state.snapshot_meta
    );

    let (dummy_tx, _rx) = UTConfig::<()>::oneshot();
    assert_eq!(
        vec![
            //
            Command::from(sm::Command::install_full_snapshot(
                Snapshot {
                    meta: SnapshotMeta {
                        last_log_id: Some(log_id(4, 1, 6)),
                        last_membership: StoredMembership::new(Some(log_id(1, 1, 1)), m1234()),
                        snapshot_id: "1-2-3-4".to_string(),
                    },
                    snapshot: Cursor::new(vec![0u8]),
                },
                LogIOId::new(Vote::new(2, 1).into_committed(), Some(log_id(4, 1, 6)))
            )),
            Command::PurgeLog { upto: log_id(4, 1, 6) },
            Command::Respond {
                when: Some(Condition::Snapshot {
                    log_id: log_id(4, 1, 6)
                }),
                resp: Respond::new(SnapshotResponse::new(curr_vote), dummy_tx),
            },
        ],
        eng.output.take_commands()
    );

    Ok(())
}
