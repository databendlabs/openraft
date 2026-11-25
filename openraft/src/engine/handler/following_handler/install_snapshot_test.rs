use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::Membership;
use crate::MembershipState;
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
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::SnapshotMetaOf;
use crate::type_config::alias::SnapshotOf;
use crate::type_config::alias::StoredMembershipOf;
use crate::type_config::alias::VoteOf;
use crate::vote::raft_vote::RaftVoteExt;

fn m12() -> Membership<u64, ()> {
    Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2}], [])
}

fn m1234() -> Membership<u64, ()> {
    Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1,2,3,4}], [])
}

fn eng() -> Engine<UTConfig> {
    let mut eng: Engine<UTConfig> = Engine::testing_default(0);
    eng.state.enable_validation(false); // Disable validation for incomplete state

    let now = UTConfig::<()>::now();
    let vote = VoteOf::<UTConfig>::new_committed(2, 1);
    eng.state.vote.update(now, Duration::from_millis(500), vote);
    eng.state.apply_progress_mut().accept(log_id(4, 1, 5));
    // Last-per-leader format: leader 2 last at 4, leader 3 last at 5, leader 4 last at 8
    eng.state.log_ids = LogIdList::new(None, vec![
        //
        log_id(2, 1, 4),
        log_id(3, 1, 5),
        log_id(4, 1, 8),
    ]);
    eng.state.snapshot_meta = SnapshotMetaOf::<UTConfig> {
        last_log_id: Some(log_id(2, 1, 2)),
        last_membership: StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 1)), m12()),
    };
    eng.state.server_state = eng.calc_server_state();

    eng
}

#[test]
fn test_install_snapshot_lt_last_snapshot() -> anyhow::Result<()> {
    // Snapshot will not be installed because new `last_log_id` is less or equal current
    // `snapshot_meta.last_log_id`. The reply waits for the flush of the logs it covers.
    let mut eng = eng();
    let vote = *eng.state.vote_ref();
    let vote_io = IOId::new(&vote);
    eng.state.log_progress_mut().try_update_all(vote_io.clone());

    let cond = eng.following_handler().install_full_snapshot(SnapshotOf::<UTConfig, ()> {
        meta: SnapshotMetaOf::<UTConfig> {
            last_log_id: Some(log_id(2, 1, 2)),
            last_membership: StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 1)), m1234()),
        },
        snapshot: (),
    });

    let io_id = IOId::new_log_io(vote.to_committed(), Some(log_id(2, 1, 2)));
    assert_eq!(Some(Condition::IOFlushed { io_id: io_id.clone() }), cond);

    assert_eq!(
        SnapshotMetaOf::<UTConfig> {
            last_log_id: Some(log_id(2, 1, 2)),
            last_membership: StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 1)), m12()),
        },
        eng.state.snapshot_meta
    );
    let accepted = eng.state.accepted_log_io();
    assert_eq!(Some(&io_id), accepted);
    let commands = eng.output.take_commands();
    assert_eq!(
        vec![Command::UpdateIOProgress {
            when: Some(Condition::IOFlushed { io_id: vote_io }),
            io_id,
        }],
        commands
    );

    Ok(())
}

#[test]
fn test_install_snapshot_lt_committed() -> anyhow::Result<()> {
    // Snapshot will not be installed because new `last_log_id` is less or equal current
    // `committed`. TODO: The snapshot should be able to be updated if
    // `new_snapshot.last_log_id > engine.snapshot_meta.last_log_id`.
    // Although in this case the state machine is not affected.
    // The reply waits for the flush of the logs it covers.
    let mut eng = eng();
    let leader = eng.state.vote_ref().to_committed();
    let heartbeat_io = IOId::new_log_io(leader.clone(), Some(log_id(2, 1, 3)));
    eng.state.log_progress_mut().try_update_all(heartbeat_io.clone());

    let cond = eng.following_handler().install_full_snapshot(SnapshotOf::<UTConfig, ()> {
        meta: SnapshotMetaOf::<UTConfig> {
            last_log_id: Some(log_id(2, 1, 4)),
            last_membership: StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 1)), m1234()),
        },
        snapshot: (),
    });

    let io_id = IOId::new_log_io(leader, Some(log_id(2, 1, 4)));
    assert_eq!(Some(Condition::IOFlushed { io_id: io_id.clone() }), cond);

    assert_eq!(
        SnapshotMetaOf::<UTConfig> {
            last_log_id: Some(log_id(2, 1, 2)),
            last_membership: StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 1)), m12()),
        },
        eng.state.snapshot_meta
    );
    let accepted = eng.state.accepted_log_io();
    assert_eq!(Some(&io_id), accepted);
    let commands = eng.output.take_commands();
    assert_eq!(
        vec![Command::UpdateIOProgress {
            when: Some(Condition::IOFlushed { io_id: heartbeat_io }),
            io_id,
        }],
        commands
    );

    Ok(())
}

/// A snapshot whose logs this node has committed is not installed, and the reply waits until
/// they are flushed: `local_committed` follows the accepted log IO. The flush of the leader's IO
/// up to the snapshot meets the condition, and so does the flush of any later IO, such as a vote.
#[test]
fn test_install_committed_snapshot_waits_for_flush() -> anyhow::Result<()> {
    let leader = Vote::new(4, 1).to_committed();
    let io = |index| IOId::new_log_io(leader.clone(), Some(log_id(4, 1, index)));
    let snapshot = SnapshotOf::<UTConfig, ()> {
        meta: SnapshotMetaOf::<UTConfig> {
            last_log_id: Some(log_id(4, 1, 5)),
            last_membership: StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 1)), m1234()),
        },
        snapshot: (),
    };
    let eng_with_flushed = |flushed: IOId<UTConfig>| {
        let mut eng = eng();
        eng.state.vote.update(
            UTConfig::<()>::now(),
            Duration::from_millis(500),
            Vote::new_committed(4, 1),
        );
        eng.state.log_progress_mut().accept(io(8));
        eng.state.log_progress_mut().submit(io(8));
        eng.state.log_progress_mut().flush(flushed);
        eng
    };

    tracing::info!("--- logs accepted up to 8 and flushed up to 3: the reply waits for the flush up to 5");
    {
        let mut eng = eng_with_flushed(io(3));
        let cond = eng.following_handler().install_full_snapshot(snapshot.clone()).unwrap();

        assert_eq!(Condition::IOFlushed { io_id: io(5) }, cond);
        assert!(!cond.is_met(eng.state.io_state()));
        assert!(eng.output.take_commands().is_empty());

        eng.state.log_progress_mut().flush(io(8));
        assert!(cond.is_met(eng.state.io_state()));
    }

    tracing::info!("--- logs already flushed up to 8: the reply is sent at once");
    {
        let mut eng = eng_with_flushed(io(8));
        let cond = eng.following_handler().install_full_snapshot(snapshot.clone()).unwrap();
        assert!(cond.is_met(eng.state.io_state()));
    }

    tracing::info!("--- a later vote flushed after the logs meets the condition too");
    {
        let mut eng = eng_with_flushed(io(3));
        let cond = eng.following_handler().install_full_snapshot(snapshot.clone()).unwrap();

        let later_vote = IOId::new(&Vote::new(5, 2));
        eng.state.log_progress_mut().accept(later_vote.clone());
        eng.state.log_progress_mut().submit(later_vote.clone());
        eng.state.log_progress_mut().flush(later_vote);
        assert!(cond.is_met(eng.state.io_state()));
    }

    Ok(())
}

/// A duplicate of a snapshot whose install is in flight is not installed again, and its reply
/// waits until the install completes: until then, no log IO covers the snapshot.
#[test]
fn test_duplicate_snapshot_waits_for_the_install_in_flight() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state.vote.update(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(4, 1),
    );
    let snapshot = SnapshotOf::<UTConfig, ()> {
        meta: SnapshotMetaOf::<UTConfig> {
            last_log_id: Some(log_id(4, 1, 6)),
            last_membership: StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 1)), m1234()),
        },
        snapshot: (),
    };
    let install_io = LogIOId::new(Vote::new(4, 1).to_committed(), Some(log_id(4, 1, 6)));

    tracing::info!("--- the first copy starts the install");
    {
        let cond = eng.following_handler().install_full_snapshot(snapshot.clone());
        assert_eq!(
            Some(Condition::Snapshot {
                log_id: log_id(4, 1, 6)
            }),
            cond
        );
        eng.output.take_commands();
    }

    tracing::info!("--- the duplicate waits for the install's log IO, which completes the install");
    {
        let cond = eng.following_handler().install_full_snapshot(snapshot).unwrap();

        assert_eq!(
            Condition::IOFlushed {
                io_id: install_io.to_io_id()
            },
            cond
        );
        assert!(eng.output.take_commands().is_empty());
        assert!(!cond.is_met(eng.state.io_state()));

        // As RaftCore does when it submits the install, and when the state machine reports it
        // done.
        eng.state.log_progress_mut().submit(install_io.to_io_id());
        assert!(!cond.is_met(eng.state.io_state()));
        eng.state.log_progress_mut().try_flush(install_io.to_io_id());
        assert!(cond.is_met(eng.state.io_state()));
    }

    Ok(())
}

#[test]
fn test_install_snapshot_not_conflict() -> anyhow::Result<()> {
    // Snapshot will be installed and there are no conflicting logs.
    let mut eng = eng();

    // The vote must cover the snapshot's last log id, as `accept_vote()` guarantees on the real
    // path.
    eng.state.vote.update(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(4, 1),
    );

    let cond = eng.following_handler().install_full_snapshot(SnapshotOf::<UTConfig, ()> {
        meta: SnapshotMetaOf::<UTConfig> {
            last_log_id: Some(log_id(4, 1, 6)),
            last_membership: StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 1)), m1234()),
        },
        snapshot: (),
    });

    assert_eq!(
        Some(Condition::Snapshot::<UTConfig> {
            log_id: log_id(4, 1, 6)
        }),
        cond
    );

    assert_eq!(
        SnapshotMetaOf::<UTConfig> {
            last_log_id: Some(log_id(4, 1, 6)),
            last_membership: StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 1)), m1234()),
        },
        eng.state.snapshot_meta
    );
    assert_eq!(Some(&log_id(4, 1, 6)), eng.state.log_ids.purged());
    assert_eq!(&[log_id(4, 1, 8)], eng.state.log_ids.key_log_ids());
    assert_eq!(Some(&log_id(4, 1, 6)), eng.state.local_committed());
    assert_eq!(
        &Arc::new(StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 1)), m1234())),
        eng.state.membership_state.committed()
    );
    assert_eq!(
        vec![
            //
            Command::from(sm::Command::install_full_snapshot(
                SnapshotOf::<UTConfig, ()> {
                    meta: SnapshotMetaOf::<UTConfig> {
                        last_log_id: Some(log_id(4, 1, 6)),
                        last_membership: StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 1)), m1234()),
                    },
                    snapshot: (),
                },
                LogIOId::new(Vote::new(4, 1).to_committed(), Some(log_id(4, 1, 6))),
            )),
            Command::PurgeLog { upto: log_id(4, 1, 6) },
        ],
        eng.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_install_snapshot_resets_purged_effective_membership() -> anyhow::Result<()> {
    // Snapshot last_membership has a lower index than the stale effective membership, but the
    // snapshot covers and purges the stale effective membership log.
    let mut eng = eng();
    let snapshot_membership = StoredMembershipOf::<UTConfig>::new(Some(log_id(5, 1, 4)), m12());

    eng.state.vote.update(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(5, 1),
    );
    eng.state.membership_state = MembershipState::new(
        Arc::new(StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 1)), m12())),
        Arc::new(StoredMembershipOf::<UTConfig>::new(Some(log_id(4, 1, 8)), m1234())),
    );
    eng.state.server_state = eng.calc_server_state();

    let cond = eng.following_handler().install_full_snapshot(SnapshotOf::<UTConfig, ()> {
        meta: SnapshotMetaOf::<UTConfig> {
            last_log_id: Some(log_id(5, 1, 9)),
            last_membership: snapshot_membership.clone(),
        },
        snapshot: (),
    });

    assert_eq!(
        Some(Condition::Snapshot::<UTConfig> {
            log_id: log_id(5, 1, 9)
        }),
        cond
    );

    let expected_membership = Arc::new(snapshot_membership);
    assert_eq!(
        MembershipState::new(expected_membership.clone(), expected_membership),
        eng.state.membership_state
    );

    Ok(())
}

#[test]
fn test_install_snapshot_resets_purged_effective_without_truncating() -> anyhow::Result<()> {
    let mut eng = eng();
    let snapshot_membership = StoredMembershipOf::<UTConfig>::new(Some(log_id(5, 1, 4)), m12());

    eng.state.vote.update(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(5, 1),
    );
    eng.state.log_ids = LogIdList::new(None, vec![
        //
        log_id(2, 1, 4),
        log_id(3, 1, 5),
        log_id(4, 1, 8),
        log_id(5, 1, 9),
    ]);
    eng.state.membership_state = MembershipState::new(
        Arc::new(StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 1)), m12())),
        Arc::new(StoredMembershipOf::<UTConfig>::new(Some(log_id(4, 1, 8)), m1234())),
    );
    eng.state.server_state = eng.calc_server_state();

    let cond = eng.following_handler().install_full_snapshot(SnapshotOf::<UTConfig, ()> {
        meta: SnapshotMetaOf::<UTConfig> {
            last_log_id: Some(log_id(5, 1, 9)),
            last_membership: snapshot_membership.clone(),
        },
        snapshot: (),
    });

    assert_eq!(
        Some(Condition::Snapshot::<UTConfig> {
            log_id: log_id(5, 1, 9)
        }),
        cond
    );

    let expected_membership = Arc::new(snapshot_membership);
    assert_eq!(
        MembershipState::new(expected_membership.clone(), expected_membership),
        eng.state.membership_state
    );
    assert_eq!(
        vec![
            Command::from(sm::Command::install_full_snapshot(
                SnapshotOf::<UTConfig, ()> {
                    meta: SnapshotMetaOf::<UTConfig> {
                        last_log_id: Some(log_id(5, 1, 9)),
                        last_membership: StoredMembershipOf::<UTConfig>::new(Some(log_id(5, 1, 4)), m12()),
                    },
                    snapshot: (),
                },
                LogIOId::new(Vote::new(5, 1).to_committed(), Some(log_id(5, 1, 9)))
            )),
            Command::PurgeLog { upto: log_id(5, 1, 9) },
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
            Vote::new_committed(5, 1),
        );
        eng.state.apply_progress_mut().accept(log_id(2, 1, 3));
        eng.state.log_ids = LogIdList::new(None, vec![
            //
            log_id(2, 1, 4),
            log_id(3, 1, 5),
            log_id(4, 1, 8),
        ]);

        eng.state.snapshot_meta = SnapshotMetaOf::<UTConfig> {
            last_log_id: Some(log_id(2, 1, 2)),
            last_membership: StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 1)), m12()),
        };

        eng.state.server_state = eng.calc_server_state();

        eng
    };

    let cond = eng.following_handler().install_full_snapshot(SnapshotOf::<UTConfig, ()> {
        meta: SnapshotMetaOf::<UTConfig> {
            last_log_id: Some(log_id(5, 1, 6)),
            last_membership: StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 1)), m1234()),
        },
        snapshot: (),
    });

    assert_eq!(
        Some(Condition::Snapshot::<UTConfig> {
            log_id: log_id(5, 1, 6)
        }),
        cond
    );

    assert_eq!(
        SnapshotMetaOf::<UTConfig> {
            last_log_id: Some(log_id(5, 1, 6)),
            last_membership: StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 1)), m1234()),
        },
        eng.state.snapshot_meta
    );
    // All logs purged by snapshot, only purged field contains the snapshot's last_log_id
    assert!(eng.state.log_ids.key_log_ids().is_empty());
    assert_eq!(Some(&log_id(5, 1, 6)), eng.state.log_ids.purged());
    assert_eq!(Some(&log_id(5, 1, 6)), eng.state.local_committed());
    assert_eq!(
        &Arc::new(StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 1)), m1234())),
        eng.state.membership_state.committed()
    );
    assert_eq!(
        vec![
            //
            Command::TruncateLog {
                after: Some(log_id(2, 1, 3))
            },
            Command::from(sm::Command::install_full_snapshot(
                SnapshotOf::<UTConfig, ()> {
                    meta: SnapshotMetaOf::<UTConfig> {
                        last_log_id: Some(log_id(5, 1, 6)),
                        last_membership: StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 1)), m1234()),
                    },
                    snapshot: (),
                },
                LogIOId::new(Vote::new(5, 1).to_committed(), Some(log_id(5, 1, 6)))
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

    eng.state.vote.update(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(100, 1),
    );

    let cond = eng.following_handler().install_full_snapshot(SnapshotOf::<UTConfig, ()> {
        meta: SnapshotMetaOf::<UTConfig> {
            last_log_id: Some(log_id(100, 1, 100)),
            last_membership: StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 1)), m1234()),
        },
        snapshot: (),
    });

    assert_eq!(
        Some(Condition::Snapshot::<UTConfig> {
            log_id: log_id(100, 1, 100)
        }),
        cond
    );

    assert_eq!(
        SnapshotMetaOf::<UTConfig> {
            last_log_id: Some(log_id(100, 1, 100)),
            last_membership: StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 1)), m1234()),
        },
        eng.state.snapshot_meta
    );
    // All logs purged by snapshot, only purged field contains the snapshot's last_log_id
    assert!(eng.state.log_ids.key_log_ids().is_empty());
    assert_eq!(Some(&log_id(100, 1, 100)), eng.state.log_ids.purged());
    assert_eq!(Some(&log_id(100, 1, 100)), eng.state.local_committed());
    assert_eq!(
        &Arc::new(StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 1)), m1234())),
        eng.state.membership_state.committed()
    );
    assert_eq!(
        &Arc::new(StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 1)), m1234())),
        eng.state.membership_state.effective()
    );
    assert_eq!(
        vec![
            Command::from(sm::Command::install_full_snapshot(
                SnapshotOf::<UTConfig, ()> {
                    meta: SnapshotMetaOf::<UTConfig> {
                        last_log_id: Some(log_id(100, 1, 100)),
                        last_membership: StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 1)), m1234()),
                    },
                    snapshot: (),
                },
                LogIOId::new(Vote::new(100, 1).to_committed(), Some(log_id(100, 1, 100)))
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

    eng.state.vote.update(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(100, 1),
    );

    let cond = eng.following_handler().install_full_snapshot(SnapshotOf::<UTConfig, ()> {
        meta: SnapshotMetaOf::<UTConfig> {
            last_log_id: Some(log_id(100, 1, 100)),
            last_membership: StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 1)), m1234()),
        },
        snapshot: (),
    });

    assert_eq!(
        Some(Condition::Snapshot::<UTConfig> {
            log_id: log_id(100, 1, 100)
        }),
        cond
    );

    assert_eq!(
        Some(&IOId::new_log_io(
            Vote::new(100, 1).to_committed(),
            Some(log_id(100, 1, 100))
        )),
        eng.state.accepted_log_io()
    );

    Ok(())
}

#[test]
#[should_panic(expected = "can not be owned by the installing leader")]
fn test_install_snapshot_beyond_leader_vote() {
    // The vote is (2,1): no leader owns a snapshot with a term-4 last log id.
    let mut eng = eng();

    let _ = eng.following_handler().install_full_snapshot(SnapshotOf::<UTConfig, ()> {
        meta: SnapshotMetaOf::<UTConfig> {
            last_log_id: Some(log_id(4, 1, 6)),
            last_membership: StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 1)), m1234()),
        },
        snapshot: (),
    });
}

#[test]
#[should_panic(expected = "contradicts locally committed logs")]
fn test_install_snapshot_conflict_with_committed() {
    // A snapshot greater than the local committed (4,1,5) but at a smaller index contradicts
    // locally committed logs.
    let mut eng = eng();

    eng.state.vote.update(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(5, 1),
    );

    let _ = eng.following_handler().install_full_snapshot(SnapshotOf::<UTConfig, ()> {
        meta: SnapshotMetaOf::<UTConfig> {
            last_log_id: Some(log_id(5, 1, 3)),
            last_membership: StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 1)), m1234()),
        },
        snapshot: (),
    });
}

#[test]
#[should_panic(expected = "contradicts locally committed logs")]
fn test_install_snapshot_conflict_at_committed_index() {
    let mut eng = eng();
    eng.state.vote.update(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(5, 1),
    );

    let _ = eng.following_handler().install_full_snapshot(SnapshotOf::<UTConfig, ()> {
        meta: SnapshotMetaOf::<UTConfig> {
            // A greater log id at the committed index identifies a different committed entry.
            last_log_id: Some(log_id(5, 1, 5)),
            last_membership: StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 1)), m1234()),
        },
        snapshot: (),
    });
}
