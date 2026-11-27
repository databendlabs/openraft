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

fn m23() -> Membership<u64, ()> {
    Membership::<u64, ()>::new_with_defaults(vec![btreeset! {2,3}], btreeset! {1,2,3})
}

fn eng() -> Engine<UTConfig> {
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
        Arc::new(StoredMembershipOf::<UTConfig>::new(Some(log_id(1, 1, 1)), m23())),
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

    let mut lh = eng.try_leader_handler()?;

    lh.transfer_leader(2);

    assert_eq!(lh.leader.get_transfer_to(), Some(&2));
    assert!(
        !lh.leader.is_transfer_expired(),
        "the transfer has an election timeout to complete"
    );

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
fn test_transfer_retry_keeps_original_deadline() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.output.take_commands();
    let mut lh = eng.try_leader_handler()?;

    tracing::info!("--- start a transfer to node 2 with a deadline that is already due");
    let deadline = {
        lh.config.timer_config.election_timeout = Duration::ZERO;
        lh.transfer_leader(2);
        lh.output.take_commands();
        lh.leader.transfer_deadline().expect("transfer sets a deadline")
    };

    tracing::info!("--- retry to node 2 with a longer timeout and keep the original deadline");
    {
        lh.config.timer_config.election_timeout = Duration::from_secs(1);
        lh.transfer_leader(2);
        let retry_deadline = lh.leader.transfer_deadline();
        let retry_broadcast = lh.output.take_commands();
        let req = TransferLeaderRequest::new(Vote::new_committed(3, 1), 2, Some(log_id(2, 1, 3)));
        assert_eq!(Some(deadline), retry_deadline);
        assert_eq!(vec![Command::BroadcastTransferLeader { req }], retry_broadcast);
    }

    tracing::info!("--- a transfer to node 3 starts a new attempt");
    {
        lh.transfer_leader(3);
        let target = lh.leader.get_transfer_to();
        let replacement_deadline = lh.leader.transfer_deadline();
        assert_eq!(Some(&3), target);
        assert!(replacement_deadline > Some(deadline));
    }

    Ok(())
}
