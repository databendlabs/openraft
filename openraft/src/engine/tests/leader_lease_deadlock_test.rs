//! Engine-level reproduction of issue #2080: liveness deadlock under a partial
//! network failure.
//!
//! Five voters with a three-way connectivity split:
//!
//! ```text
//!        a -------- b            isolated
//!         \        /
//!          \      /
//!           bridge  <-- old leader heartbeats
//! ```
//!
//! The old Leader reaches only the `bridge`; the quorum {a, bridge, b} stays
//! connected. Two engines stand in for the two decisive nodes:
//!
//! - node-1 `bridge`: still hears the old Leader's heartbeat; every heartbeat renews its
//!   follower-side leader lease via `Leased::touch`;
//! - node-3: the old Leader; its quorum-ack lease is dead (2 acks < quorum 3) yet it keeps
//!   broadcasting heartbeats.
//!
//! Node-0 `a`'s election is simulated with a term-2 `VoteRequest` sent to the
//! bridge.

use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::Membership;
use crate::MembershipState;
use crate::core::ServerState;
use crate::engine::Engine;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::impls::Vote;
use crate::raft::LogSegment;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::StoredMembershipOf;
use crate::type_config::alias::VoteOf;
use crate::utime::Leased;

fn m01234() -> Membership<u64, ()> {
    Membership::<u64, ()>::new_with_defaults(vec![btreeset! {0, 1, 2, 3, 4}], [])
}

fn membership_state() -> MembershipState<crate::engine::testing::UtClid, u64, ()> {
    MembershipState::new(
        Arc::new(StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 3, 0)), m01234())),
        Arc::new(StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 3, 0)), m01234())),
    )
}

/// Node ids follow the issue topology: 0 = `a`, 1 = `bridge`, 3 = the old Leader.
fn bridge() -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(1);
    eng.state.enable_validation(false);

    eng.state.vote = Leased::new(
        UTConfig::<()>::now(),
        eng.config.timer_config.leader_lease,
        Vote::new_committed(1, 3),
    );
    eng.state.log_ids.append(log_id(1, 3, 0));
    eng.state.membership_state = membership_state();
    eng.state.server_state = eng.calc_server_state();

    assert_eq!(ServerState::Follower, eng.state.server_state);
    eng
}

fn old_leader() -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(3);
    eng.state.enable_validation(false);

    // A Leader never renews the lease on its own `state.vote`; the live lease is
    // the quorum-ack lease tracked by `Leader`.
    eng.state.vote = Leased::new(
        UTConfig::<()>::now(),
        Duration::from_millis(0),
        Vote::new_committed(1, 3),
    );
    eng.state.log_ids.append(log_id(1, 3, 0));
    eng.state.membership_state = membership_state();
    eng.state.server_state = eng.calc_server_state();
    eng.testing_new_leader();

    assert_eq!(ServerState::Leader, eng.state.server_state);
    eng
}

/// One heartbeat from the old Leader: an AppendEntries with the Leader's
/// committed vote and no entries.
fn heartbeat(
    bridge: &mut Engine<UTConfig>,
    leader_vote: VoteOf<UTConfig>,
    leader_last: LogIdOf<UTConfig>,
) -> LogIdOf<UTConfig> {
    let acked = bridge
        .append_entries(&leader_vote, LogSegment::new(Some(leader_last), vec![]))
        .expect("the bridge accepts the old Leader's heartbeat");

    assert!(
        !bridge.state.vote.is_expired(UTConfig::<()>::now(), Duration::from_millis(0)),
        "the heartbeat renewed the bridge's leader lease"
    );
    assert_eq!(leader_vote, *bridge.state.vote_ref());

    acked.expect("an empty segment acks prev_log_id")
}

#[test]
fn test_2080_old_leader_heartbeat_blocks_connected_quorum() -> anyhow::Result<()> {
    let mut bridge = bridge();
    let mut old_leader = old_leader();
    let leader_vote = Vote::new_committed(1, 3);
    let leader_last = log_id(1, 3, 0);

    // Phase: the old Leader's heartbeat renews the bridge's follower-side lease.
    {
        tracing::info!("old leader heartbeats the bridge: lease renewed by touch, vote unchanged");
        heartbeat(&mut bridge, leader_vote, leader_last);
    }

    // Phase: the connected quorum campaigns; the bridge rejects while its lease is fresh.
    {
        tracing::info!("node-a campaigns at term-2; bridge rejects by follower-side leader lease");
        let resp = bridge.handle_vote_req(VoteRequest {
            vote: Vote::new(2, 0),
            last_log_id: Some(leader_last),
            leadership_transfer: false,
        });
        assert_eq!(VoteResponse::new(leader_vote, Some(leader_last), false), resp);
    }

    // Phase: the old Leader hears acks only from the bridge: no quorum, quorum-ack lease dead.
    //
    // The last quorum acknowledgement happened at `stale_ack` (election time: the
    // leader itself and two followers acked the blank-entry replication). Since
    // then only the bridge acks, which is not a quorum, so `quorum_accepted` stays
    // at the stale value.
    {
        tracing::info!("old leader: no quorum acks since election: quorum-ack lease expired, it cannot commit");
        let lease = old_leader.config.timer_config.leader_lease;
        let stale_ack = UTConfig::<()>::now() - lease - lease;
        let leader = old_leader.leader.as_mut().unwrap();
        for node_id in [0, 1, 2] {
            leader.clock_progress.update_entry_with(&node_id, |entry| entry.val = Some(stale_ack));
        }

        assert_eq!(Some(stale_ack), leader.last_quorum_acked_time());
        assert!(!leader.is_lease_valid(lease));
        assert!(
            leader.should_suppress_heartbeat(lease),
            "the lease has been expired for more than one leader_lease"
        );
    }

    // Phase: the old Leader must not broadcast heartbeats once the quorum-ack lease is dead.
    {
        tracing::info!("quorum-ack lease expired: send_heartbeat is gated and emits no command");

        let sent = old_leader.try_leader_handler()?.send_heartbeat(false);

        assert!(!sent, "heartbeat must be suppressed by the expired quorum-ack lease");
        assert_eq!(0, old_leader.output.take_commands().len());
    }

    // Phase: the cycle never ends: each heartbeat renews the bridge, each renewed
    // lease rejects the quorum's election, and the old Leader can never commit.
    {
        tracing::info!("heartbeat renews the bridge again; the term-2 election is rejected again");

        heartbeat(&mut bridge, leader_vote, leader_last);

        let resp = bridge.handle_vote_req(VoteRequest {
            vote: Vote::new(2, 0),
            last_log_id: Some(leader_last),
            leadership_transfer: false,
        });
        assert_eq!(VoteResponse::new(leader_vote, Some(leader_last), false), resp);
    }

    Ok(())
}
