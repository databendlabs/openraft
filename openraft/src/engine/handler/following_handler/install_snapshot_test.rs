use std::sync::Arc;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::engine::testing::UTCfg;
use crate::engine::CEngine;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::raft_state::LogStateReader;
use crate::testing::log_id;
use crate::EffectiveMembership;
use crate::Membership;
use crate::MetricsChangeFlags;
use crate::SnapshotMeta;
use crate::StoredMembership;

fn m12() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {1,2}], None)
}

fn m1234() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {1,2,3,4}], None)
}

fn eng() -> CEngine<UTCfg> {
    let mut eng = Engine::default();
    eng.state.enable_validate = false; // Disable validation for incomplete state

    eng.state.committed = Some(log_id(4, 5));
    eng.state.log_ids = LogIdList::new(vec![
        //
        log_id(2, 2),
        log_id(3, 5),
        log_id(4, 6),
        log_id(4, 8),
    ]);
    eng.state.snapshot_meta = SnapshotMeta {
        last_log_id: Some(log_id(2, 2)),
        last_membership: StoredMembership::new(Some(log_id(1, 1)), m12()),
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

    eng.following_handler().install_snapshot(SnapshotMeta {
        last_log_id: Some(log_id(2, 2)),
        last_membership: StoredMembership::new(Some(log_id(1, 1)), m1234()),
        snapshot_id: "1-2-3-4".to_string(),
    });

    assert_eq!(
        SnapshotMeta {
            last_log_id: Some(log_id(2, 2)),
            last_membership: StoredMembership::new(Some(log_id(1, 1)), m12()),
            snapshot_id: "1-2-3-4".to_string(),
        },
        eng.state.snapshot_meta
    );

    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: false,
            cluster: false,
        },
        eng.output.metrics_flags
    );

    assert_eq!(
        vec![Command::CancelSnapshot {
            snapshot_meta: SnapshotMeta {
                last_log_id: Some(log_id(2, 2)),
                last_membership: StoredMembership::new(Some(log_id(1, 1)), m1234()),
                snapshot_id: "1-2-3-4".to_string(),
            }
        }],
        eng.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_install_snapshot_lt_committed() -> anyhow::Result<()> {
    // Snapshot will not be installed because new `last_log_id` is less or equal current
    // `committed`. TODO: The snapshot should be able to be updated if
    // `new_snapshot.last_log_id > engine.snapshot_meta.last_log_id`.
    // Although in this case the state machine is not affected.
    let mut eng = eng();

    eng.following_handler().install_snapshot(SnapshotMeta {
        last_log_id: Some(log_id(4, 5)),
        last_membership: StoredMembership::new(Some(log_id(1, 1)), m1234()),
        snapshot_id: "1-2-3-4".to_string(),
    });

    assert_eq!(
        SnapshotMeta {
            last_log_id: Some(log_id(2, 2)),
            last_membership: StoredMembership::new(Some(log_id(1, 1)), m12()),
            snapshot_id: "1-2-3-4".to_string(),
        },
        eng.state.snapshot_meta
    );

    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: false,
            cluster: false,
        },
        eng.output.metrics_flags
    );

    assert_eq!(
        vec![Command::CancelSnapshot {
            snapshot_meta: SnapshotMeta {
                last_log_id: Some(log_id(4, 5)),
                last_membership: StoredMembership::new(Some(log_id(1, 1)), m1234()),
                snapshot_id: "1-2-3-4".to_string(),
            }
        }],
        eng.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_install_snapshot_not_conflict() -> anyhow::Result<()> {
    // Snapshot will be installed and there are no conflicting logs.
    let mut eng = eng();

    eng.following_handler().install_snapshot(SnapshotMeta {
        last_log_id: Some(log_id(4, 6)),
        last_membership: StoredMembership::new(Some(log_id(1, 1)), m1234()),
        snapshot_id: "1-2-3-4".to_string(),
    });

    assert_eq!(
        SnapshotMeta {
            last_log_id: Some(log_id(4, 6)),
            last_membership: StoredMembership::new(Some(log_id(1, 1)), m1234()),
            snapshot_id: "1-2-3-4".to_string(),
        },
        eng.state.snapshot_meta
    );
    assert_eq!(&[log_id(4, 6), log_id(4, 8)], eng.state.log_ids.key_log_ids());
    assert_eq!(Some(&log_id(4, 6)), eng.state.committed());
    assert_eq!(
        &Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m1234())),
        eng.state.membership_state.committed()
    );

    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: true,
            cluster: true,
        },
        eng.output.metrics_flags
    );

    assert_eq!(
        vec![
            Command::UpdateMembership {
                membership: Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m1234()))
            },
            //
            Command::InstallSnapshot {
                snapshot_meta: SnapshotMeta {
                    last_log_id: Some(log_id(4, 6)),
                    last_membership: StoredMembership::new(Some(log_id(1, 1)), m1234()),
                    snapshot_id: "1-2-3-4".to_string(),
                }
            },
            Command::PurgeLog { upto: log_id(4, 6) },
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
        let mut eng = CEngine::<UTCfg>::default();
        eng.state.enable_validate = false; // Disable validation for incomplete state

        eng.state.committed = Some(log_id(2, 3));
        eng.state.log_ids = LogIdList::new(vec![
            //
            log_id(2, 2),
            log_id(3, 5),
            log_id(4, 6),
            log_id(4, 8),
        ]);

        eng.state.snapshot_meta = SnapshotMeta {
            last_log_id: Some(log_id(2, 2)),
            last_membership: StoredMembership::new(Some(log_id(1, 1)), m12()),
            snapshot_id: "1-2-3-4".to_string(),
        };

        eng.state.server_state = eng.calc_server_state();

        eng
    };

    eng.following_handler().install_snapshot(SnapshotMeta {
        last_log_id: Some(log_id(5, 6)),
        last_membership: StoredMembership::new(Some(log_id(1, 1)), m1234()),
        snapshot_id: "1-2-3-4".to_string(),
    });

    assert_eq!(
        SnapshotMeta {
            last_log_id: Some(log_id(5, 6)),
            last_membership: StoredMembership::new(Some(log_id(1, 1)), m1234()),
            snapshot_id: "1-2-3-4".to_string(),
        },
        eng.state.snapshot_meta
    );
    assert_eq!(&[log_id(5, 6)], eng.state.log_ids.key_log_ids());
    assert_eq!(Some(&log_id(5, 6)), eng.state.committed());
    assert_eq!(
        &Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m1234())),
        eng.state.membership_state.committed()
    );

    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: true,
            cluster: true,
        },
        eng.output.metrics_flags
    );

    assert_eq!(
        vec![
            Command::DeleteConflictLog { since: log_id(2, 4) },
            Command::UpdateMembership {
                membership: Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m1234()))
            },
            //
            Command::InstallSnapshot {
                snapshot_meta: SnapshotMeta {
                    last_log_id: Some(log_id(5, 6)),
                    last_membership: StoredMembership::new(Some(log_id(1, 1)), m1234()),
                    snapshot_id: "1-2-3-4".to_string(),
                }
            },
            Command::PurgeLog { upto: log_id(5, 6) },
        ],
        eng.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_install_snapshot_advance_last_log_id() -> anyhow::Result<()> {
    // Snapshot will be installed and there are no conflicting logs.
    let mut eng = eng();

    eng.following_handler().install_snapshot(SnapshotMeta {
        last_log_id: Some(log_id(100, 100)),
        last_membership: StoredMembership::new(Some(log_id(1, 1)), m1234()),
        snapshot_id: "1-2-3-4".to_string(),
    });

    assert_eq!(
        SnapshotMeta {
            last_log_id: Some(log_id(100, 100)),
            last_membership: StoredMembership::new(Some(log_id(1, 1)), m1234()),
            snapshot_id: "1-2-3-4".to_string(),
        },
        eng.state.snapshot_meta
    );
    assert_eq!(&[log_id(100, 100)], eng.state.log_ids.key_log_ids());
    assert_eq!(Some(&log_id(100, 100)), eng.state.committed());
    assert_eq!(
        &Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m1234())),
        eng.state.membership_state.committed()
    );
    assert_eq!(
        &Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m1234())),
        eng.state.membership_state.effective()
    );

    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: true,
            cluster: true,
        },
        eng.output.metrics_flags
    );

    assert_eq!(
        vec![
            Command::UpdateMembership {
                membership: Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m1234()))
            },
            //
            Command::InstallSnapshot {
                snapshot_meta: SnapshotMeta {
                    last_log_id: Some(log_id(100, 100)),
                    last_membership: StoredMembership::new(Some(log_id(1, 1)), m1234()),
                    snapshot_id: "1-2-3-4".to_string(),
                }
            },
            Command::PurgeLog { upto: log_id(100, 100) },
        ],
        eng.output.take_commands()
    );

    Ok(())
}
