use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;

use crate::EffectiveMembership;
use crate::Membership;
use crate::MembershipState;
use crate::RaftState;
use crate::Vote;
use crate::engine::testing::UTConfig;
use crate::engine::testing::log_id;
use crate::raft_state::IOId;
use crate::raft_state::io_state::log_io_id::LogIOId;
use crate::type_config::TypeConfigExt;
use crate::utime::Leased;
use crate::vote::raft_vote::RaftVoteExt;

fn m01() -> Membership<UTConfig> {
    Membership::new_with_defaults(vec![btreeset! {0,1}], [])
}

fn m23() -> Membership<UTConfig> {
    Membership::new_with_defaults(vec![btreeset! {2,3}], [])
}

#[allow(clippy::field_reassign_with_default)]
fn new_state() -> RaftState<UTConfig> {
    let mut state = RaftState::default();

    state.vote = Leased::new(
        UTConfig::<()>::now(),
        Duration::from_millis(500),
        Vote::new_committed(2, 1),
    );
    state.apply_progress_mut().accept(log_id(1, 1, 1));
    state.membership_state = MembershipState::new(
        Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
        Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23())),
    );
    state
}

#[test]
fn test_update_committed_none() -> anyhow::Result<()> {
    let mut state = new_state();
    let committed_vote = state.vote_ref().into_committed();

    state.update_committed(LogIOId::new(committed_vote, None));

    assert_eq!(Some(&log_id(1, 1, 1)), state.committed());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23())),
        ),
        state.membership_state
    );
    assert_eq!(Some(&log_id(1, 1, 1)), state.io_state.apply_progress.accepted());
    assert_eq!(None, state.io_state.apply_progress.submitted());

    Ok(())
}

#[test]
fn test_update_committed_ge_accepted() -> anyhow::Result<()> {
    let mut state = new_state();
    let committed_vote = state.vote_ref().into_committed();
    state.log_progress_mut().accept(IOId::new_log_io(committed_vote.clone(), Some(log_id(1, 1, 2))));

    state.update_committed(LogIOId::new(committed_vote, Some(log_id(2, 1, 3))));

    // Local committed is min(accepted, cluster_committed) = min(log_id(1,1,2), log_id(2,1,3)) =
    // log_id(1,1,2)
    assert_eq!(Some(&log_id(1, 1, 2)), state.committed());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())),
            Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23())),
        ),
        state.membership_state
    );
    assert_eq!(Some(&log_id(1, 1, 2)), state.io_state.apply_progress.accepted());
    assert_eq!(None, state.io_state.apply_progress.submitted());

    Ok(())
}

#[test]
fn test_update_committed_le_accepted() -> anyhow::Result<()> {
    let mut state = new_state();
    let committed_vote = state.vote_ref().into_committed();
    state.log_progress_mut().accept(IOId::new_log_io(committed_vote.clone(), Some(log_id(3, 1, 4))));

    state.update_committed(LogIOId::new(committed_vote, Some(log_id(2, 1, 3))));

    // Local committed is min(accepted, cluster_committed) = min(log_id(3,1,4), log_id(2,1,3)) =
    // log_id(2,1,3)
    assert_eq!(Some(&log_id(2, 1, 3)), state.committed());
    assert_eq!(
        MembershipState::new(
            Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23())),
            Arc::new(EffectiveMembership::new(Some(log_id(2, 1, 3)), m23()))
        ),
        state.membership_state
    );
    assert_eq!(Some(&log_id(2, 1, 3)), state.io_state.apply_progress.accepted());
    assert_eq!(None, state.io_state.apply_progress.submitted());

    Ok(())
}

#[test]
fn test_update_committed_local_vote_lags_cluster_vote() -> anyhow::Result<()> {
    let mut state = new_state();
    let committed_vote = state.vote_ref().into_committed();
    state.log_progress_mut().accept(IOId::new_log_io(committed_vote.clone(), Some(log_id(2, 1, 3))));

    let higher_vote = Vote::new_committed(3, 2).into_committed();
    let cluster_committed = LogIOId::new(higher_vote.clone(), Some(log_id(3, 2, 4)));

    state.update_committed(cluster_committed.clone());

    // Local committed must not advance until an IO accepted under the new leader vote arrives.
    assert_eq!(Some(&log_id(1, 1, 1)), state.committed());
    assert_eq!(
        Some(cluster_committed),
        state.io_state.cluster_committed.value().cloned()
    );
    assert_eq!(Some(&log_id(1, 1, 1)), state.io_state.apply_progress.accepted());
    assert_eq!(None, state.io_state.apply_progress.submitted());

    Ok(())
}

#[test]
fn test_update_committed_advances_after_local_vote_catches_up() -> anyhow::Result<()> {
    let mut state = new_state();
    let committed_vote = state.vote_ref().into_committed();
    state.log_progress_mut().accept(IOId::new_log_io(committed_vote.clone(), Some(log_id(2, 1, 3))));

    let higher_vote = Vote::new_committed(3, 2).into_committed();
    let cluster_committed = LogIOId::new(higher_vote.clone(), Some(log_id(3, 2, 4)));

    // Commit notification arrives before logs from the new leader; no local advancement yet.
    state.update_committed(cluster_committed.clone());
    assert_eq!(Some(&log_id(1, 1, 1)), state.committed());

    // Later the follower accepts the log written by the new leader.
    state.log_progress_mut().accept(IOId::new_log_io(higher_vote.clone(), Some(log_id(3, 2, 4))));

    // Re-evaluating committed now that votes match should advance local committed.
    state.update_committed(cluster_committed.clone());

    assert_eq!(Some(&log_id(3, 2, 4)), state.committed());
    assert_eq!(Some(&log_id(3, 2, 4)), state.io_state.apply_progress.accepted());
    assert_eq!(None, state.io_state.apply_progress.submitted());

    Ok(())
}
