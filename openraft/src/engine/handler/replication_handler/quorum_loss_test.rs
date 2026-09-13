use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::Membership;
use crate::ServerState;
use crate::Vote;
use crate::engine::Engine;
use crate::engine::testing::UTConfig;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::StoredMembershipOf;
use crate::utime::Leased;

const LEASE: Duration = Duration::from_millis(100);
const GRACE: Duration = Duration::from_millis(20);

fn membership(
    voters: Vec<std::collections::BTreeSet<u64>>,
    learners: impl IntoIterator<Item = u64>,
) -> Membership<u64, ()> {
    Membership::new_with_defaults(voters, learners)
}

fn leader_engine(membership: Membership<u64, ()>) -> Engine<UTConfig> {
    let mut eng = Engine::testing_default(1);
    eng.config.timer_config.leader_lease = LEASE;
    eng.config.quorum_loss_grace = Some(GRACE);
    eng.state
        .membership_state
        .set_effective(Arc::new(StoredMembershipOf::<UTConfig>::new(None, membership)));
    eng.state.vote = Leased::new(UTConfig::<()>::now(), LEASE, Vote::new_committed(3, 1));
    eng.state.server_state = ServerState::Leader;
    eng.testing_new_leader();
    eng.output.clear_commands();
    eng
}

fn stream_id(eng: &Engine<UTConfig>, target: u64) -> crate::progress::stream_id::StreamId {
    eng.leader_ref().unwrap().progress.try_get(&target).unwrap().data.stream_id
}

#[test]
fn test_response_updates_quorum_loss_deadline() {
    tracing::info!("--- a fresh voter response advances the quorum-loss deadline");
    {
        let mut eng = leader_engine(membership(vec![btreeset! {1, 2}], []));
        let sending_time = UTConfig::<()>::now();
        let stream_id = stream_id(&eng, 2);

        eng.replication_handler().try_update_leader_clock(stream_id, 2, sending_time);

        let leader = eng.leader_ref().unwrap();
        assert_eq!(Some(sending_time), leader.last_quorum_acked_time());
        assert_eq!(Some(sending_time + LEASE + GRACE), leader.quorum_loss_retire_at());
    }

    tracing::info!("--- an already expired response updates history but not the deadline");
    {
        let mut eng = leader_engine(membership(vec![btreeset! {1, 2}], []));
        let deadline = eng.leader_ref().unwrap().quorum_loss_retire_at();
        let sending_time = UTConfig::<()>::now() - LEASE;
        let stream_id = stream_id(&eng, 2);

        eng.replication_handler().try_update_leader_clock(stream_id, 2, sending_time);

        let leader = eng.leader_ref().unwrap();
        assert_eq!(Some(sending_time), leader.last_quorum_acked_time());
        assert_eq!(deadline, leader.quorum_loss_retire_at());
    }
}

#[test]
fn test_membership_change_resets_quorum_loss_deadline_only_for_voters() {
    let mut eng = leader_engine(membership(vec![btreeset! {1, 2}], btreeset! {3}));
    let sending_time = UTConfig::<()>::now();
    let stream_id = stream_id(&eng, 2);
    eng.replication_handler().try_update_leader_clock(stream_id, 2, sending_time);
    let deadline = eng.leader_ref().unwrap().quorum_loss_retire_at();

    tracing::info!("--- changing only learners preserves the deadline and voter evidence");
    {
        eng.state.membership_state.set_effective(Arc::new(StoredMembershipOf::<UTConfig>::new(
            None,
            membership(vec![btreeset! {1, 2}], btreeset! {3, 4}),
        )));
        eng.replication_handler().rebuild_progresses();

        let leader = eng.leader_ref().unwrap();
        assert_eq!(Some(sending_time), leader.last_quorum_acked_time());
        assert_eq!(deadline, leader.quorum_loss_retire_at());
    }

    tracing::info!("--- changing the voter quorum starts a complete observation window");
    {
        eng.state.membership_state.set_effective(Arc::new(StoredMembershipOf::<UTConfig>::new(
            None,
            membership(vec![btreeset! {1, 2}, btreeset! {1, 2, 3}], btreeset! {4}),
        )));

        let before = UTConfig::<()>::now();
        eng.replication_handler().rebuild_progresses();
        let after = UTConfig::<()>::now();

        let leader = eng.leader_ref().unwrap();
        assert_eq!(Some(sending_time), leader.last_quorum_acked_time());
        let deadline = leader.quorum_loss_retire_at().unwrap();
        assert!(deadline >= before + LEASE + GRACE);
        assert!(deadline <= after + LEASE + GRACE);
    }
}
