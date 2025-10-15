use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::EffectiveMembership;
use crate::Membership;
use crate::StoredMembership;
use crate::Vote;
use crate::core::sm;
use crate::engine::Command;
use crate::engine::Condition;
use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::raft_state::IOId;
use crate::raft_state::io_state::log_io_id::LogIOId;
use crate::storage::Snapshot;
use crate::storage::SnapshotMeta;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::VoteOf;
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

    let now = UTConfig::<()>::now();
    let vote = VoteOf::<UTConfig>::new_committed(2, 1);
    eng.state.vote.update(now, Duration::from_millis(500), vote);
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
fn test_install_snapshot_lt_last_snapshot() -> anyhow::Result<()> {
    // Snapshot will not be installed because new `last_log_id` is less or equal current
    // `snapshot_meta.last_log_id`.
    let mut eng = eng();

    let cond = eng.following_handler().install_full_snapshot(Snapshot {
        meta: SnapshotMeta {
            last_log_id: Some(log_id(2, 1, 2)),
            last_membership: StoredMembership::new(Some(log_id(1, 1, 1)), m1234()),
            snapshot_id: "1-2-3-4".to_string(),
        },
        snapshot: Cursor::new(vec![0u8]),
    });

    assert_eq!(None, cond);

    assert_eq!(
        SnapshotMeta {
            last_log_id: Some(log_id(2, 1, 2)),
            last_membership: StoredMembership::new(Some(log_id(1, 1, 1)), m12()),
            snapshot_id: "1-2-3-4".to_string(),
        },
        eng.state.snapshot_meta
    );
    assert!(eng.output.take_commands().is_empty());

    Ok(())
}

#[test]
fn test_install_snapshot_lt_committed() -> anyhow::Result<()> {
    // Snapshot will not be installed because new `last_log_id` is less or equal current
    // `committed`. TODO: The snapshot should be able to be updated if
    // `new_snapshot.last_log_id > engine.snapshot_meta.last_log_id`.
    // Although in this case the state machine is not affected.
    let mut eng = eng();

    let cond = eng.following_handler().install_full_snapshot(Snapshot {
        meta: SnapshotMeta {
            last_log_id: Some(log_id(4, 1, 5)),
            last_membership: StoredMembership::new(Some(log_id(1, 1, 1)), m1234()),
            snapshot_id: "1-2-3-4".to_string(),
        },
        snapshot: Cursor::new(vec![0u8]),
    });

    assert_eq!(None, cond);

    assert_eq!(
        SnapshotMeta {
            last_log_id: Some(log_id(2, 1, 2)),
            last_membership: StoredMembership::new(Some(log_id(1, 1, 1)), m12()),
            snapshot_id: "1-2-3-4".to_string(),
        },
        eng.state.snapshot_meta
    );
    assert!(eng.output.take_commands().is_empty());

    Ok(())
}

#[test]
fn test_install_snapshot_not_conflict() -> anyhow::Result<()> {
    // Snapshot will be installed and there are no conflicting logs.
    let mut eng = eng();

    let cond = eng.following_handler().install_full_snapshot(Snapshot {
        meta: SnapshotMeta {
            last_log_id: Some(log_id(4, 1, 6)),
            last_membership: StoredMembership::new(Some(log_id(1, 1, 1)), m1234()),
            snapshot_id: "1-2-3-4".to_string(),
        },
        snapshot: Cursor::new(vec![0u8]),
    });

    assert_eq!(
        Some(Condition::Snapshot {
            log_id: log_id(4, 1, 6)
        }),
        cond
    );

    assert_eq!(
        SnapshotMeta {
            last_log_id: Some(log_id(4, 1, 6)),
            last_membership: StoredMembership::new(Some(log_id(1, 1, 1)), m1234()),
            snapshot_id: "1-2-3-4".to_string(),
        },
        eng.state.snapshot_meta
    );
    assert_eq!(&[log_id(4, 1, 6), log_id(4, 1, 8)], eng.state.log_ids.key_log_ids());
    assert_eq!(Some(&log_id(4, 1, 6)), eng.state.committed());
    assert_eq!(
        &Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m1234())),
        eng.state.membership_state.committed()
    );
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
                LogIOId::new(Vote::new(2, 1).into_committed(), Some(log_id(4, 1, 6))),
            )),
            Command::PurgeLog { upto: log_id(4, 1, 6) },
        ],
        eng.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_install_snapshot_conflict() -> anyhow::Result<()> {
    // Snapshot will be installed, all non-committed log will be deleted.
    // And there should be no conflicting logs left.
    let mut eng = {
        let mut eng = Engine::<UTConfig>::testing_default(0);
        eng.state.enable_validation(false); // Disable validation for incomplete state

        eng.state.vote.update(
            UTConfig::<()>::now(),
            Duration::from_millis(500),
            Vote::new_committed(2, 1),
        );
        eng.state.apply_progress_mut().accept(log_id(2, 1, 3));
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
    };

    let cond = eng.following_handler().install_full_snapshot(Snapshot {
        meta: SnapshotMeta {
            last_log_id: Some(log_id(5, 1, 6)),
            last_membership: StoredMembership::new(Some(log_id(1, 1, 1)), m1234()),
            snapshot_id: "1-2-3-4".to_string(),
        },
        snapshot: Cursor::new(vec![0u8]),
    });

    assert_eq!(
        Some(Condition::Snapshot {
            log_id: log_id(5, 1, 6)
        }),
        cond
    );

    assert_eq!(
        SnapshotMeta {
            last_log_id: Some(log_id(5, 1, 6)),
            last_membership: StoredMembership::new(Some(log_id(1, 1, 1)), m1234()),
            snapshot_id: "1-2-3-4".to_string(),
        },
        eng.state.snapshot_meta
    );
    assert_eq!(&[log_id(5, 1, 6)], eng.state.log_ids.key_log_ids());
    assert_eq!(Some(&log_id(5, 1, 6)), eng.state.committed());
    assert_eq!(
        &Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m1234())),
        eng.state.membership_state.committed()
    );
    assert_eq!(
        vec![
            //
            Command::TruncateLog { since: log_id(2, 1, 4) },
            Command::from(sm::Command::install_full_snapshot(
                Snapshot {
                    meta: SnapshotMeta {
                        last_log_id: Some(log_id(5, 1, 6)),
                        last_membership: StoredMembership::new(Some(log_id(1, 1, 1)), m1234()),
                        snapshot_id: "1-2-3-4".to_string(),
                    },
                    snapshot: Cursor::new(vec![0u8]),
                },
                LogIOId::new(Vote::new(2, 1).into_committed(), Some(log_id(5, 1, 6)))
            )),
            Command::PurgeLog { upto: log_id(5, 1, 6) },
        ],
        eng.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_install_snapshot_advance_last_log_id() -> anyhow::Result<()> {
    // Snapshot will be installed and there are no conflicting logs.
    let mut eng = eng();

    let cond = eng.following_handler().install_full_snapshot(Snapshot {
        meta: SnapshotMeta {
            last_log_id: Some(log_id(100, 1, 100)),
            last_membership: StoredMembership::new(Some(log_id(1, 1, 1)), m1234()),
            snapshot_id: "1-2-3-4".to_string(),
        },
        snapshot: Cursor::new(vec![0u8]),
    });

    assert_eq!(
        Some(Condition::Snapshot {
            log_id: log_id(100, 1, 100)
        }),
        cond
    );

    assert_eq!(
        SnapshotMeta {
            last_log_id: Some(log_id(100, 1, 100)),
            last_membership: StoredMembership::new(Some(log_id(1, 1, 1)), m1234()),
            snapshot_id: "1-2-3-4".to_string(),
        },
        eng.state.snapshot_meta
    );
    assert_eq!(&[log_id(100, 1, 100)], eng.state.log_ids.key_log_ids());
    assert_eq!(Some(&log_id(100, 1, 100)), eng.state.committed());
    assert_eq!(
        &Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m1234())),
        eng.state.membership_state.committed()
    );
    assert_eq!(
        &Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m1234())),
        eng.state.membership_state.effective()
    );
    assert_eq!(
        vec![
            Command::from(sm::Command::install_full_snapshot(
                Snapshot {
                    meta: SnapshotMeta {
                        last_log_id: Some(log_id(100, 1, 100)),
                        last_membership: StoredMembership::new(Some(log_id(1, 1, 1)), m1234()),
                        snapshot_id: "1-2-3-4".to_string(),
                    },
                    snapshot: Cursor::new(vec![0u8]),
                },
                LogIOId::new(Vote::new(2, 1).into_committed(), Some(log_id(100, 1, 100)))
            )),
            Command::PurgeLog {
                upto: log_id(100, 1, 100)
            },
        ],
        eng.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_install_snapshot_update_accepted() -> anyhow::Result<()> {
    // Snapshot will be installed and `accepted` should be updated.
    let mut eng = eng();

    let cond = eng.following_handler().install_full_snapshot(Snapshot {
        meta: SnapshotMeta {
            last_log_id: Some(log_id(100, 1, 100)),
            last_membership: StoredMembership::new(Some(log_id(1, 1, 1)), m1234()),
            snapshot_id: "1-2-3-4".to_string(),
        },
        snapshot: Cursor::new(vec![0u8]),
    });

    assert_eq!(
        Some(Condition::Snapshot {
            log_id: log_id(100, 1, 100)
        }),
        cond
    );

    assert_eq!(
        Some(&IOId::new_log_io(
            Vote::new(2, 1).into_committed(),
            Some(log_id(100, 1, 100))
        )),
        eng.state.accepted_log_io()
    );

    Ok(())
}
