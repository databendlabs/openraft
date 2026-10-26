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
use crate::raft::TransferLeaderRequest;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::StoredMembershipOf;
use crate::utime::Leased;

fn m1() -> Membership<u64, ()> {
    Membership::<u64, ()>::new_with_defaults(vec![btreeset! {1}], btreeset! {2})
}

fn m23() -> Membership<u64, ()> {
    Membership::<u64, ()>::new_with_defaults(vec![btreeset! {2,3}], btreeset! {1,2,3})
}

fn eng(membership: Membership<u64, ()>) -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(0);
    eng.state.enable_validation(false); // Disable validation for incomplete state

    eng.config.id = 1;
    eng.state.vote = Leased::new(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(3, 1),
    );
    eng.state.log_ids.append(log_id(1, 1, 1));
    eng.state.log_ids.append(log_id(2, 1, 3));
    eng.state.membership_state = MembershipState::new(
        Arc::new(StoredMembershipOf::<UTConfig>::new(
            Some(log_id(1, 1, 1)),
            membership.clone(),
        )),
        Arc::new(StoredMembershipOf::<UTConfig>::new(Some(log_id(2, 1, 3)), membership)),
    );
    eng.testing_new_leader();
    eng.state.server_state = eng.calc_server_state();

    eng
}

#[test]
fn test_leader_send_heartbeat() -> anyhow::Result<()> {
    let mut eng = eng(m23());
    eng.output.take_commands();

    let mut lh = eng.try_leader_handler()?;

    lh.transfer_leader(2);

    let transfer_to = lh.leader.get_transfer_to();
    assert_eq!(Some(&2), transfer_to);

    let lease_info = lh.state.vote.lease_info();
    assert_eq!(lease_info.1, Duration::default());
    assert_eq!(lease_info.2, false);

    assert_eq!(
        vec![
            //
            Command::BroadcastTransferLeader {
                req: TransferLeaderRequest::new(Vote::new_committed(3, 1), 2, Some(log_id(2, 1, 3))),
            },
        ],
        eng.output.take_commands()
    );

    Ok(())
}

#[test]
fn test_self_quorum_leader_aborts_expired_transfer() -> anyhow::Result<()> {
    let mut eng = eng(m1());
    let mut lh = eng.try_leader_handler()?;
    let election_timeout = lh.config.timer_config.election_timeout;

    lh.transfer_leader(2);

    // A transfer younger than an election timeout goes on.
    lh.abort_expired_transfer();
    let transfer_to = lh.leader.get_transfer_to();
    assert_eq!(Some(&2), transfer_to);

    // A transfer as old as an election timeout ends, and the leader alone holds its lease again.
    let started_at = UTConfig::<()>::now() - election_timeout;
    lh.leader.transfer_to = Some((2, started_at));
    lh.abort_expired_transfer();
    let transfer_to = lh.leader.get_transfer_to();
    assert_eq!(None, transfer_to);
    let lease_valid = lh.is_lease_valid();
    assert!(lease_valid);

    Ok(())
}

#[test]
fn test_leader_needing_acks_keeps_expired_transfer() -> anyhow::Result<()> {
    let mut eng = eng(m23());
    let mut lh = eng.try_leader_handler()?;
    let election_timeout = lh.config.timer_config.election_timeout;

    let started_at = UTConfig::<()>::now() - election_timeout;
    lh.leader.transfer_to = Some((2, started_at));
    lh.abort_expired_transfer();
    let transfer_to = lh.leader.get_transfer_to();
    assert_eq!(Some(&2), transfer_to);

    Ok(())
}
