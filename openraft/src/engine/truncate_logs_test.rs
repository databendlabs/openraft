use std::sync::Arc;

use maplit::btreeset;

use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::EffectiveMembership;
use crate::LeaderId;
use crate::LogId;
use crate::Membership;
use crate::MembershipState;
use crate::MetricsChangeFlags;
use crate::ServerState;

fn log_id(term: u64, index: u64) -> LogId<u64> {
    LogId::<u64> {
        leader_id: LeaderId { term, node_id: 1 },
        index,
    }
}

fn m01() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {0,1}], None)
}

fn m12() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {1,2}], None)
}

fn m23() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {2,3}], None)
}

fn eng() -> Engine<u64, ()> {
    let mut eng = Engine::<u64, ()> {
        id: 2,
        ..Default::default()
    };
    eng.state.server_state = ServerState::Follower;
    eng.state.log_ids = LogIdList::new(vec![
        log_id(2, 2), //
        log_id(4, 4),
        log_id(4, 6),
    ]);
    eng.state.membership_state.committed = Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01()));
    eng.state.membership_state.effective = Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23()));
    eng
}

#[test]
fn test_truncate_logs_since_3() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.truncate_logs(3);

    assert_eq!(Some(log_id(2, 2)), eng.state.last_log_id(),);
    assert_eq!(&[log_id(2, 2)], eng.state.log_ids.key_log_ids());
    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: true,
            cluster: true,
        },
        eng.metrics_flags
    );
    assert_eq!(
        MembershipState {
            committed: Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
            effective: Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01()))
        },
        eng.state.membership_state
    );
    assert_eq!(ServerState::Learner, eng.state.server_state);

    assert_eq!(
        vec![
            //
            Command::DeleteConflictLog { since: log_id(2, 3) },
            Command::UpdateMembership {
                membership: Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01()))
            },
            Command::UpdateServerState {
                server_state: ServerState::Learner
            },
        ],
        eng.commands
    );

    Ok(())
}

#[test]
fn test_truncate_logs_since_4() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.truncate_logs(4);

    assert_eq!(Some(log_id(2, 3)), eng.state.last_log_id(),);
    assert_eq!(&[log_id(2, 2), log_id(2, 3)], eng.state.log_ids.key_log_ids());
    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: true,
            cluster: false,
        },
        eng.metrics_flags
    );
    assert_eq!(
        MembershipState {
            committed: Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
            effective: Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23()))
        },
        eng.state.membership_state
    );
    assert_eq!(ServerState::Follower, eng.state.server_state);

    assert_eq!(vec![Command::DeleteConflictLog { since: log_id(4, 4) }], eng.commands);

    Ok(())
}

#[test]
fn test_truncate_logs_since_5() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.truncate_logs(5);

    assert_eq!(Some(log_id(4, 4)), eng.state.last_log_id(),);
    assert_eq!(&[log_id(2, 2), log_id(4, 4)], eng.state.log_ids.key_log_ids());
    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: true,
            cluster: false,
        },
        eng.metrics_flags
    );

    assert_eq!(vec![Command::DeleteConflictLog { since: log_id(4, 5) }], eng.commands);

    Ok(())
}

#[test]
fn test_truncate_logs_since_6() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.truncate_logs(6);

    assert_eq!(Some(log_id(4, 5)), eng.state.last_log_id(),);
    assert_eq!(
        &[log_id(2, 2), log_id(4, 4), log_id(4, 5)],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: true,
            cluster: false,
        },
        eng.metrics_flags
    );

    assert_eq!(vec![Command::DeleteConflictLog { since: log_id(4, 6) }], eng.commands);

    Ok(())
}

#[test]
fn test_truncate_logs_since_7() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.truncate_logs(7);

    assert_eq!(Some(log_id(4, 6)), eng.state.last_log_id(),);
    assert_eq!(
        &[log_id(2, 2), log_id(4, 4), log_id(4, 6)],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: false,
            cluster: false,
        },
        eng.metrics_flags
    );

    assert!(eng.commands.is_empty());

    Ok(())
}

#[test]
fn test_truncate_logs_since_8() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.truncate_logs(8);

    assert_eq!(Some(log_id(4, 6)), eng.state.last_log_id(),);
    assert_eq!(
        &[log_id(2, 2), log_id(4, 4), log_id(4, 6)],
        eng.state.log_ids.key_log_ids()
    );
    assert_eq!(
        MetricsChangeFlags {
            replication: false,
            local_data: false,
            cluster: false,
        },
        eng.metrics_flags
    );

    assert!(eng.commands.is_empty());

    Ok(())
}

#[test]
fn test_truncate_logs_revert_effective_membership() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state.membership_state = MembershipState {
        committed: Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m01())),
        effective: Arc::new(EffectiveMembership::new(Some(log_id(4, 4)), m12())),
    };

    eng.truncate_logs(4);

    assert_eq!(Some(log_id(2, 3)), eng.state.last_log_id(),);
    assert_eq!(&[log_id(2, 2), log_id(2, 3)], eng.state.log_ids.key_log_ids());
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
            //
            Command::DeleteConflictLog { since: log_id(4, 4) },
            Command::UpdateMembership {
                membership: Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m01()))
            },
            Command::UpdateServerState {
                server_state: ServerState::Learner
            },
        ],
        eng.commands
    );

    Ok(())
}
