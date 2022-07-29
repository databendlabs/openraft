use std::sync::Arc;

use maplit::btreeset;

use super::calc_purge_upto_test::TestNodeType;
use crate::core::ServerState;
use crate::engine::Command;
use crate::engine::Engine;
use crate::EffectiveMembership;
use crate::LeaderId;
use crate::LogId;
use crate::Membership;
use crate::MembershipState;
use crate::MetricsChangeFlags;

crate::declare_raft_types!(
    pub(crate) Foo: D=(), R=(), NodeType = TestNodeType
);

fn log_id(term: u64, index: u64) -> LogId<u64> {
    LogId::<u64> {
        leader_id: LeaderId { term, node_id: 1 },
        index,
    }
}

fn m01() -> Membership<TestNodeType> {
    Membership::<TestNodeType>::new(vec![btreeset! {0,1}], None)
}

fn m23() -> Membership<TestNodeType> {
    Membership::<TestNodeType>::new(vec![btreeset! {2,3}], None)
}

fn m34() -> Membership<TestNodeType> {
    Membership::<TestNodeType>::new(vec![btreeset! {3,4}], None)
}

fn eng() -> Engine<TestNodeType> {
    let mut eng = Engine::<TestNodeType> {
        id: 2, // make it a member
        ..Default::default()
    };
    eng.state.server_state = ServerState::Follower;
    eng.state.membership_state.committed = Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01()));
    eng.state.membership_state.effective = Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23()));
    eng
}

#[test]
fn test_update_committed_membership_at_index_0() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.update_committed_membership(EffectiveMembership::new(Some(log_id(0, 0)), m34()));

    assert_eq!(
        MembershipState {
            committed: Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
            effective: Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23()))
        },
        eng.state.membership_state
    );
    assert_eq!(ServerState::Follower, eng.state.server_state);

    assert_eq!(
        MetricsChangeFlags {
            leader: false,
            other_metrics: false
        },
        eng.metrics_flags
    );

    assert_eq!(0, eng.commands.len());

    Ok(())
}

#[test]
fn test_update_committed_membership_at_index_2() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.update_committed_membership(EffectiveMembership::new(Some(log_id(1, 2)), m34()));

    assert_eq!(
        MembershipState {
            committed: Arc::new(EffectiveMembership::new(Some(log_id(1, 2)), m34())),
            effective: Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23()))
        },
        eng.state.membership_state
    );
    assert_eq!(ServerState::Follower, eng.state.server_state);

    assert_eq!(
        MetricsChangeFlags {
            leader: false,
            other_metrics: false
        },
        eng.metrics_flags
    );

    assert_eq!(0, eng.commands.len());

    Ok(())
}

#[test]
fn test_update_committed_membership_at_index_3() -> anyhow::Result<()> {
    // replace effective membership
    let mut eng = eng();

    eng.update_committed_membership(EffectiveMembership::new(Some(log_id(3, 3)), m34()));

    assert_eq!(
        MembershipState {
            committed: Arc::new(EffectiveMembership::new(Some(log_id(3, 3)), m34())),
            effective: Arc::new(EffectiveMembership::new(Some(log_id(3, 3)), m34()))
        },
        eng.state.membership_state
    );
    assert_eq!(ServerState::Learner, eng.state.server_state);

    assert_eq!(
        MetricsChangeFlags {
            leader: false,
            other_metrics: true
        },
        eng.metrics_flags
    );

    assert_eq!(
        vec![
            Command::UpdateMembership {
                membership: Arc::new(EffectiveMembership::new(Some(log_id(3, 3)), m34())),
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
fn test_update_committed_membership_at_index_4() -> anyhow::Result<()> {
    // replace effective membership
    let mut eng = eng();

    eng.update_committed_membership(EffectiveMembership::new(Some(log_id(3, 4)), m34()));

    assert_eq!(
        MembershipState {
            committed: Arc::new(EffectiveMembership::new(Some(log_id(3, 4)), m34())),
            effective: Arc::new(EffectiveMembership::new(Some(log_id(3, 4)), m34()))
        },
        eng.state.membership_state
    );
    assert_eq!(ServerState::Learner, eng.state.server_state);

    assert_eq!(
        MetricsChangeFlags {
            leader: false,
            other_metrics: true
        },
        eng.metrics_flags
    );

    assert_eq!(
        vec![
            Command::UpdateMembership {
                membership: Arc::new(EffectiveMembership::new(Some(log_id(3, 4)), m34())),
            },
            Command::UpdateServerState {
                server_state: ServerState::Learner
            },
        ],
        eng.commands
    );

    Ok(())
}
