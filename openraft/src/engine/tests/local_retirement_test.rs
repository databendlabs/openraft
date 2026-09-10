use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::Membership;
use crate::ServerState;
use crate::Vote;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::testing::UTConfig;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::LeaderIdOf;
use crate::type_config::alias::StoredMembershipOf;
use crate::utime::Leased;
use crate::vote::RaftLeaderId;

fn leader_engine() -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(1);
    eng.state.enable_validation(false);
    eng.state.membership_state.set_effective(Arc::new(StoredMembershipOf::<UTConfig>::new(
        None,
        Membership::new_with_defaults(vec![btreeset! {1, 2}], []),
    )));
    eng.state.vote = Leased::new(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(3, 1),
    );
    eng.state.server_state = ServerState::Leader;
    eng.testing_new_leader();
    eng.output.clear_commands();
    eng
}

#[test]
fn test_retire_leader_locally() {
    let mut eng = leader_engine();
    let retired_for = LeaderIdOf::<UTConfig>::new(3, 1);

    assert!(eng.retire_leader_locally());

    assert_eq!(Some(retired_for), eng.state.locally_retired_for);
    assert_eq!(&Vote::new_committed(3, 1), eng.state.vote_ref());
    assert!(eng.leader_ref().is_none());
    assert!(eng.candidate_ref().is_none());
    assert!(eng.pre_candidate_ref().is_none());
    assert_eq!(ServerState::Follower, eng.state.server_state);
    assert_eq!(
        vec![
            Command::FailPendingReads,
            Command::SaveLocalRetirement { retired_for },
            Command::CloseReplicationStreams,
        ],
        eng.output.take_commands()
    );
}

#[test]
fn test_retire_leader_locally_ignores_stale_leader_session() {
    let mut eng = leader_engine();
    eng.state.vote = Leased::new(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(4, 1),
    );

    assert!(!eng.retire_leader_locally());

    assert_eq!(None, eng.state.locally_retired_for);
    assert!(eng.leader_ref().is_some());
    assert!(eng.output.take_commands().is_empty());
}

#[test]
fn test_try_retire_leader_on_quorum_loss() {
    let lease = Duration::from_millis(100);
    let grace = Duration::from_millis(20);
    let started = UTConfig::<()>::now();

    tracing::info!("--- no deadline leaves quorum-loss retirement disabled");
    {
        let mut eng = leader_engine();
        assert!(!eng.try_retire_leader_on_quorum_loss(started + lease + grace));
        assert!(eng.leader_ref().is_some());
        assert!(eng.output.take_commands().is_empty());
    }

    tracing::info!("--- an unrecovered quorum loss retires the Leader at the deadline");
    {
        let mut eng = leader_engine();
        eng.config.timer_config.leader_lease = lease;
        eng.leader_mut().unwrap().reset_quorum_loss_deadline(started, lease, grace);

        assert!(!eng.try_retire_leader_on_quorum_loss(started + lease + grace - Duration::from_nanos(1)));
        assert!(eng.try_retire_leader_on_quorum_loss(started + lease + grace));
        assert!(eng.leader_ref().is_none());
    }

    tracing::info!("--- a fresh quorum acknowledgement cancels retirement eligibility");
    {
        let mut eng = leader_engine();
        eng.config.timer_config.leader_lease = lease;
        eng.leader_mut().unwrap().reset_quorum_loss_deadline(started, lease, grace);
        eng.leader_mut().unwrap().update_clock(&2, started + grace + Duration::from_millis(1));

        assert!(!eng.try_retire_leader_on_quorum_loss(started + lease + grace));
        assert!(eng.leader_ref().is_some());
        assert!(eng.output.take_commands().is_empty());
    }
}
