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
    let now = UTConfig::<()>::now();
    eng.leader.as_mut().unwrap().update_clock(&2, now);
    eng.leader.as_mut().unwrap().update_clock(&3, now);

    eng.output.take_commands();

    // A heartbeat is a normal AppendEntries RPC if there are pending data to send.
    {
        eng.try_leader_handler()?.send_heartbeat(false);
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
        eng.try_leader_handler()?.send_heartbeat(false);
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

    Ok(())
}

#[test]
fn test_leader_send_heartbeat_suppressed_by_expired_quorum_ack_lease() -> anyhow::Result<()> {
    // A leader that has been without quorum acknowledgement for more than one
    // extra leader_lease must not broadcast heartbeats.
    let mut eng = eng();
    let lease = eng.config.timer_config.leader_lease;
    let stale_ack = UTConfig::<()>::now() - lease - lease - Duration::from_millis(1);
    let leader = eng.leader.as_mut().unwrap();
    for node_id in [2, 3] {
        leader.clock_progress.update_entry_with(&node_id, |entry| entry.val = Some(stale_ack));
    }
    eng.output.take_commands();

    let sent = eng.try_leader_handler()?.send_heartbeat(false);

    assert!(!sent);
    assert_eq!(0, eng.output.take_commands().len());

    Ok(())
}

#[test]
fn test_leader_send_heartbeat_not_suppressed_within_one_extra_lease() -> anyhow::Result<()> {
    // A transient lease lapse shorter than one extra leader_lease is healed by the
    // next heartbeat round: the heartbeat is not suppressed.
    let mut eng = eng();
    let lease = eng.config.timer_config.leader_lease;
    let stale_ack = UTConfig::<()>::now() - lease;
    let leader = eng.leader.as_mut().unwrap();
    for node_id in [2, 3] {
        leader.clock_progress.update_entry_with(&node_id, |entry| entry.val = Some(stale_ack));
    }
    eng.output.take_commands();

    let sent = eng.try_leader_handler()?.send_heartbeat(false);

    assert!(sent);
    assert_eq!(1, eng.output.take_commands().len());

    Ok(())
}
