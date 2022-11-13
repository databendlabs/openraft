use std::sync::Arc;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::EffectiveMembership;
use crate::LeaderId;
use crate::LogId;
use crate::Membership;
use crate::MetricsChangeFlags;
use crate::SnapshotMeta;

fn log_id(term: u64, index: u64) -> LogId<u64> {
    LogId::<u64> {
        leader_id: LeaderId { term, node_id: 1 },
        index,
    }
}

fn m12() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {1,2}], None)
}

fn m1234() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {1,2,3,4}], None)
}

fn eng() -> Engine<u64, ()> {
    let mut eng = Engine::<u64, ()> {
        snapshot_meta: SnapshotMeta {
            last_log_id: Some(log_id(2, 2)),
            last_membership: EffectiveMembership::new(Some(log_id(1, 1)), m12()),
            snapshot_id: "1-2-3-4".to_string(),
        },
        ..Default::default()
    };

    eng.state.committed = Some(log_id(4, 5));
    eng.state.log_ids = LogIdList::new(vec![
        //
        log_id(2, 2),
        log_id(3, 5),
        log_id(4, 6),
        log_id(4, 8),
    ]);

    eng
}

#[test]
fn test_install_snapshot_lt_last_snapshot() -> anyhow::Result<()> {
    // Snapshot will not be installed because new `last_log_id` is less or equal current `snapshot_meta.last_log_id`.
    let mut eng = eng();

    eng.install_snapshot(SnapshotMeta {
        last_log_id: Some(log_id(2, 2)),
        last_membership: EffectiveMembership::new(Some(log_id(1, 1)), m1234()),
        snapshot_id: "1-2-3-4".to_string(),
    });

    assert_eq!(
        SnapshotMeta {
            last_log_id: Some(log_id(2, 2)),
            last_membership: EffectiveMembership::new(Some(log_id(1, 1)), m12()),
            snapshot_id: "1-2-3-4".to_string(),
        },
        eng.snapshot_meta
    );

    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: false,
            cluster: false,
        },
        eng.metrics_flags
    );

    Ok(())
}

#[test]
fn test_install_snapshot_lt_committed() -> anyhow::Result<()> {
    // Snapshot will not be installed because new `last_log_id` is less or equal current `committed`.
    // TODO: The snapshot should be able to be updated if `new_snapshot.last_log_id > engine.snapshot_meta.last_log_id`.
    //       Although in this case the state machine is not affected.
    let mut eng = eng();

    eng.install_snapshot(SnapshotMeta {
        last_log_id: Some(log_id(4, 5)),
        last_membership: EffectiveMembership::new(Some(log_id(1, 1)), m1234()),
        snapshot_id: "1-2-3-4".to_string(),
    });

    assert_eq!(
        SnapshotMeta {
            last_log_id: Some(log_id(2, 2)),
            last_membership: EffectiveMembership::new(Some(log_id(1, 1)), m12()),
            snapshot_id: "1-2-3-4".to_string(),
        },
        eng.snapshot_meta
    );

    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: false,
            cluster: false,
        },
        eng.metrics_flags
    );

    Ok(())
}

#[test]
fn test_install_snapshot_not_conflict() -> anyhow::Result<()> {
    // Snapshot will be installed and there are no conflicting logs.
    let mut eng = eng();

    eng.install_snapshot(SnapshotMeta {
        last_log_id: Some(log_id(4, 6)),
        last_membership: EffectiveMembership::new(Some(log_id(1, 1)), m1234()),
        snapshot_id: "1-2-3-4".to_string(),
    });

    assert_eq!(
        SnapshotMeta {
            last_log_id: Some(log_id(4, 6)),
            last_membership: EffectiveMembership::new(Some(log_id(1, 1)), m1234()),
            snapshot_id: "1-2-3-4".to_string(),
        },
        eng.snapshot_meta
    );
    assert_eq!(&[log_id(4, 6), log_id(4, 8)], eng.state.log_ids.key_log_ids());
    assert_eq!(Some(log_id(4, 6)), eng.state.committed);
    assert_eq!(
        Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m1234())),
        eng.state.membership_state.committed
    );

    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: true,
            cluster: true,
        },
        eng.metrics_flags
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
                    last_membership: EffectiveMembership::new(Some(log_id(1, 1)), m1234()),
                    snapshot_id: "1-2-3-4".to_string(),
                }
            },
            Command::PurgeLog { upto: log_id(4, 6) },
        ],
        eng.commands
    );

    Ok(())
}

#[test]
fn test_install_snapshot_conflict() -> anyhow::Result<()> {
    // Snapshot will be installed, all non-committed log will be deleted.
    // And there should be no conflicting logs left.
    let mut eng = {
        let mut eng = Engine::<u64, ()> {
            snapshot_meta: SnapshotMeta {
                last_log_id: Some(log_id(2, 2)),
                last_membership: EffectiveMembership::new(Some(log_id(1, 1)), m12()),
                snapshot_id: "1-2-3-4".to_string(),
            },
            ..Default::default()
        };

        eng.state.committed = Some(log_id(2, 3));
        eng.state.log_ids = LogIdList::new(vec![
            //
            log_id(2, 2),
            log_id(3, 5),
            log_id(4, 6),
            log_id(4, 8),
        ]);

        eng
    };

    eng.install_snapshot(SnapshotMeta {
        last_log_id: Some(log_id(5, 6)),
        last_membership: EffectiveMembership::new(Some(log_id(1, 1)), m1234()),
        snapshot_id: "1-2-3-4".to_string(),
    });

    assert_eq!(
        SnapshotMeta {
            last_log_id: Some(log_id(5, 6)),
            last_membership: EffectiveMembership::new(Some(log_id(1, 1)), m1234()),
            snapshot_id: "1-2-3-4".to_string(),
        },
        eng.snapshot_meta
    );
    assert_eq!(&[log_id(5, 6)], eng.state.log_ids.key_log_ids());
    assert_eq!(Some(log_id(5, 6)), eng.state.committed);
    assert_eq!(
        Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m1234())),
        eng.state.membership_state.committed
    );

    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: true,
            cluster: true,
        },
        eng.metrics_flags
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
                    last_membership: EffectiveMembership::new(Some(log_id(1, 1)), m1234()),
                    snapshot_id: "1-2-3-4".to_string(),
                }
            },
            Command::PurgeLog { upto: log_id(5, 6) },
        ],
        eng.commands
    );

    Ok(())
}

#[test]
fn test_install_snapshot_advance_last_log_id() -> anyhow::Result<()> {
    // Snapshot will be installed and there are no conflicting logs.
    let mut eng = eng();

    eng.install_snapshot(SnapshotMeta {
        last_log_id: Some(log_id(100, 100)),
        last_membership: EffectiveMembership::new(Some(log_id(1, 1)), m1234()),
        snapshot_id: "1-2-3-4".to_string(),
    });

    assert_eq!(
        SnapshotMeta {
            last_log_id: Some(log_id(100, 100)),
            last_membership: EffectiveMembership::new(Some(log_id(1, 1)), m1234()),
            snapshot_id: "1-2-3-4".to_string(),
        },
        eng.snapshot_meta
    );
    assert_eq!(&[log_id(100, 100)], eng.state.log_ids.key_log_ids());
    assert_eq!(Some(log_id(100, 100)), eng.state.committed);
    assert_eq!(
        Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m1234())),
        eng.state.membership_state.committed
    );
    assert_eq!(
        Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m1234())),
        eng.state.membership_state.effective
    );

    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: true,
            cluster: true,
        },
        eng.metrics_flags
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
                    last_membership: EffectiveMembership::new(Some(log_id(1, 1)), m1234()),
                    snapshot_id: "1-2-3-4".to_string(),
                }
            },
            Command::PurgeLog { upto: log_id(100, 100) },
        ],
        eng.commands
    );

    Ok(())
}
