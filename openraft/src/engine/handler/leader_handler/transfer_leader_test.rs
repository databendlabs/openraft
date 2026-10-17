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

    assert_eq!(lh.leader.transfer_to, Some(2));

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
fn test_resend_transfer_leader_when_target_catches_up() -> anyhow::Result<()> {
    let mut eng = eng();

    eng.try_leader_handler()?.transfer_leader(2);
    eng.output.take_commands();

    let req = TransferLeaderRequest::new(Vote::new_committed(3, 1), 2, Some(log_id(2, 1, 3)));

    // Another voter catching up does not resend.
    eng.replication_handler().update_matching(3, Some(log_id(2, 1, 3)), None);
    assert_eq!(0, eng.output.take_commands().len());

    // The target still lacks the leader's last log id.
    eng.replication_handler().update_matching(2, Some(log_id(2, 1, 2)), None);
    assert_eq!(0, eng.output.take_commands().len());

    // The target reaches the leader's last log id: resend the same request.
    eng.replication_handler().update_matching(2, Some(log_id(2, 1, 3)), None);
    assert_eq!(
        vec![Command::BroadcastTransferLeader { req }],
        eng.output.take_commands()
    );

    // Resend only once, when the target reaches it.
    eng.replication_handler().update_matching(2, Some(log_id(2, 1, 3)), None);
    assert_eq!(0, eng.output.take_commands().len());

    Ok(())
}
