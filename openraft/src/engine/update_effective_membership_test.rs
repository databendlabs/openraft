use std::sync::Arc;

use maplit::btreeset;

use crate::core::ServerState;
use crate::engine::Command;
use crate::engine::Engine;
use crate::progress::Progress;
use crate::EffectiveMembership;
use crate::LeaderId;
use crate::LogId;
use crate::Membership;
use crate::MembershipState;
use crate::MetricsChangeFlags;
use crate::Vote;

crate::declare_raft_types!(
    pub(crate) Foo: D=(), R=(), NodeId=u64
);

fn log_id(term: u64, index: u64) -> LogId<u64> {
    LogId::<u64> {
        leader_id: LeaderId { term, node_id: 1 },
        index,
    }
}

fn m01() -> Membership<u64> {
    Membership::<u64>::new(vec![btreeset! {0,1}], None)
}

fn m23() -> Membership<u64> {
    Membership::<u64>::new(vec![btreeset! {2,3}], None)
}

fn m34() -> Membership<u64> {
    Membership::<u64>::new(vec![btreeset! {3,4}], None)
}

fn eng() -> Engine<u64> {
    let mut eng = Engine::<u64> {
        id: 2, // make it a member
        ..Default::default()
    };
    eng.state.server_state = ServerState::Follower;
    eng.state.membership_state.committed = Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01()));
    eng.state.membership_state.effective = Arc::new(EffectiveMembership::new(Some(log_id(2, 3)), m23()));
    eng
}

#[test]
fn test_update_effective_membership_at_index_0_is_allowed() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.update_effective_membership(&log_id(0, 0), &m34());

    assert_eq!(
        MembershipState {
            committed: Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
            effective: Arc::new(EffectiveMembership::new(Some(log_id(0, 0)), m34()))
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
                membership: Arc::new(EffectiveMembership::new(Some(log_id(0, 0)), m34())),
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
fn test_update_effective_membership_for_leader() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state.server_state = ServerState::Leader;
    // Make it a real leader: voted for itself and vote is committed.
    eng.state.vote = Vote::new_committed(2, 2);
    eng.state.new_leader();

    eng.update_effective_membership(&log_id(3, 4), &m34());

    assert_eq!(
        MembershipState {
            committed: Arc::new(EffectiveMembership::new(Some(log_id(1, 1)), m01())),
            effective: Arc::new(EffectiveMembership::new(Some(log_id(3, 4)), m34()))
        },
        eng.state.membership_state
    );
    assert_eq!(
        ServerState::Leader,
        eng.state.server_state,
        "Leader wont be affected by membership change"
    );

    assert_eq!(
        MetricsChangeFlags {
            leader: false,
            other_metrics: true
        },
        eng.metrics_flags
    );

    assert_eq!(
        vec![Command::UpdateMembership {
            membership: Arc::new(EffectiveMembership::new(Some(log_id(3, 4)), m34())),
        },],
        eng.commands
    );

    assert!(
        eng.state.leader.unwrap().progress.get(&4).is_none(),
        "exists, but it is a None"
    );

    Ok(())
}
