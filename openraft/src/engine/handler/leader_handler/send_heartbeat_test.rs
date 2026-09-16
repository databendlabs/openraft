use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::Membership;
use crate::MembershipState;
use crate::Vote;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::replication::ReplicationSessionId;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::StoredMembershipOf;
use crate::utime::Leased;
use crate::vote::raft_vote::RaftVoteExt;

fn m01() -> Membership<u64, ()> {
    Membership::<u64, ()>::new_with_defaults(vec![btreeset! {0,1}], [])
}

fn m23() -> Membership<u64, ()> {
    Membership::<u64, ()>::new_with_defaults(vec![btreeset! {2,3}], btreeset! {1,2,3})
}

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(0);
    eng.state.enable_validation(false); // Disable validation for incomplete state

    eng.config.id = 1;
    eng.state.apply_progress_mut().accept(log_id(0, 1, 0));
    eng.state.vote = Leased::new(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(3, 1),
    );
    eng.state.log_ids.append(log_id(1, 1, 1));
    eng.state.log_ids.append(log_id(2, 1, 3));
    eng.state.membership_state = MembershipState::new(
        Arc::new(StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 1)), m01())),
        Arc::new(StoredMembershipOf::<UTConfig>::new(Some(log_id(2, 1, 3)), m23())),
    );
    eng.testing_new_leader();
    eng.state.server_state = eng.calc_server_state();

    eng
}

#[test]
fn test_leader_send_heartbeat() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.output.take_commands();

    // A heartbeat is a normal AppendEntries RPC if there are pending data to send.
    {
        assert!(eng.try_leader_handler()?.send_heartbeat(false));
        assert_eq!(
            vec![
                //
                Command::BroadcastHeartbeat {
                    session_id: ReplicationSessionId::new(Vote::new(3, 1).to_committed(), Some(log_id(2, 1, 3))),
                    bypass_min_interval: false,
                },
            ],
            eng.output.take_commands()
        );
    }

    // Heartbeat will be resent
    {
        eng.output.clear_commands();
        assert!(eng.try_leader_handler()?.send_heartbeat(false));
        assert_eq!(
            vec![
                //
                Command::BroadcastHeartbeat {
                    session_id: ReplicationSessionId::new(Vote::new(3, 1).to_committed(), Some(log_id(2, 1, 3))),
                    bypass_min_interval: false,
                },
            ],
            eng.output.take_commands()
        );
    }

    tracing::info!("Suppress a heartbeat during a quorum-loss quiet period");
    {
        eng.output.clear_commands();
        let leader_lease = Duration::from_millis(300);
        eng.config.timer_config.leader_lease = leader_lease;
        eng.config.quorum_loss_probe_interval = Some(Duration::from_millis(600));
        let activity_at = UTConfig::<()>::now() - leader_lease;
        eng.state.vote = Leased::new(activity_at, leader_lease, Vote::new_committed(3, 1));
        let sent = eng.try_leader_handler()?.send_heartbeat(true);
        assert!(!sent);
        let commands = eng.output.take_commands();
        assert!(commands.is_empty());
    }

    Ok(())
}

#[test]
fn test_quorum_loss_heartbeat_periods() -> anyhow::Result<()> {
    let mut eng = eng();
    let leader_lease = Duration::from_millis(300);
    let period = Duration::from_millis(600);
    eng.config.timer_config.leader_lease = leader_lease;
    eng.config.quorum_loss_probe_interval = Some(period);

    let activity_at = UTConfig::<()>::now();
    eng.state.vote = Leased::new(activity_at, leader_lease, Vote::new_committed(3, 1));

    tracing::info!("Use the current Vote time before the first quorum acknowledgement");
    {
        let quorum_acked_at = eng.leader.as_ref().unwrap().last_quorum_acked_time();
        assert_eq!(None, quorum_acked_at);

        let before_lease_expiry = activity_at + leader_lease - Duration::from_millis(1);
        let allowed = eng.try_leader_handler()?.heartbeat_is_allowed(before_lease_expiry);
        assert!(allowed);

        let quiet_at = activity_at + leader_lease;
        let allowed = eng.try_leader_handler()?.heartbeat_is_allowed(quiet_at);
        assert!(!allowed);

        let send_at = quiet_at + period;
        let allowed = eng.try_leader_handler()?.heartbeat_is_allowed(send_at);
        assert!(allowed);

        let quiet_again_at = send_at + period;
        let allowed = eng.try_leader_handler()?.heartbeat_is_allowed(quiet_again_at);
        assert!(!allowed);
    }

    tracing::info!("Prefer a quorum acknowledgement over the older Vote time");
    {
        let acked_at = activity_at + period + period + period;
        let leader = eng.leader.as_mut().unwrap();
        leader.update_clock(&2, acked_at);
        leader.update_clock(&3, acked_at);

        let check_at = acked_at + leader_lease - Duration::from_millis(1);
        let allowed = eng.try_leader_handler()?.heartbeat_is_allowed(check_at);
        assert!(allowed);
    }

    Ok(())
}
