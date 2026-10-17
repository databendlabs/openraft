use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::ChangeMembers;
use openraft::Config;
use openraft::EntryPayload;
use openraft::Membership;
use openraft::ServerState;
use openraft::StepDownPolicy;
use openraft::async_runtime::WatchReceiver;
use openraft::errors::ChangeMembershipError;
use openraft::errors::ClientWriteError;
use openraft::errors::ForwardToLeader;
use openraft::errors::LeaderNotInJointMembership;
use openraft::errors::RaftError;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// Election, heartbeat and ticking stay off, and a removed leader does not step down by itself,
/// so it stays a leader outside the membership for as long as a test needs.
fn config() -> Result<Arc<Config>> {
    let config = Config {
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        removed_leader_step_down: StepDownPolicy::Never,
        ..Default::default()
    }
    .validate()?;
    Ok(Arc::new(config))
}

/// A leader that removed itself stays a leader until it steps down, outside the membership. Every
/// membership change asked of it meanwhile answers `ForwardToLeader` naming no leader, and writes
/// nothing.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn removed_leader_refuses_membership_changes() -> Result<()> {
    let mut router = RaftRouter::new(config()?);

    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {3}).await?;
    let leader = router.get_raft_handle(&0)?;

    tracing::info!(log_index, "--- node 0 removes itself and stays the leader");
    {
        leader.change_membership([1, 2], false).await?;
        log_index += 2;

        leader.wait(timeout()).applied_index(Some(log_index), "the removal is applied").await?;
        assert_eq!(ServerState::Leader, leader.metrics().borrow_watched().state);
    }

    tracing::info!(log_index, "--- every membership API is forwarded, to no leader");
    {
        let want = RaftError::APIError(ClientWriteError::ForwardToLeader(ForwardToLeader::empty()));

        let err = leader.change_membership(ChangeMembers::RemoveNodes(btreeset! {3}), false).await.unwrap_err();
        assert_eq!(want, err, "change_membership");

        let err = leader.add_learner(4, (), false).await.unwrap_err();
        assert_eq!(want, err, "add_learner");

        let proposed = Membership::new_with_defaults(vec![btreeset! {1,2}], []);
        let err = leader.append_membership(proposed, EntryPayload::Blank, []).await.unwrap_err();
        assert_eq!(want, err, "append_membership");
    }

    tracing::info!(log_index, "--- the refused changes wrote nothing");
    {
        let metrics = leader.metrics().borrow_watched().clone();
        assert_eq!(Some(log_index), metrics.last_log_index);
        assert_eq!(ServerState::Leader, metrics.state);
    }

    Ok(())
}

/// From the joint membership `[{0,1,2}, {1,2,3}]`, leader 0 is asked for voters `{1,2}`.
///
/// `change_membership()` would bridge through `[{1,2,3}, {1,2}]`, which leaves node 0 out of both
/// voter sets, so node 0 could not write the second step once that joint membership commits. The
/// change is refused and writes nothing. With node 0 retained as a learner, it is in the joint
/// membership, and the same change completes.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn change_from_joint_does_not_leave_out_the_leader() -> Result<()> {
    let mut router = RaftRouter::new(config()?);

    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {3}).await?;
    let leader = router.get_raft_handle(&0)?;

    tracing::info!(
        log_index,
        "--- leave the cluster in a joint membership, as an interrupted change would"
    );
    {
        let joint = Membership::new_with_defaults(vec![btreeset! {0,1,2}, btreeset! {1,2,3}], []);
        leader.append_membership(joint, EntryPayload::Blank, []).await?;
        log_index += 1;

        leader.wait(timeout()).applied_index(Some(log_index), "the joint membership is applied").await?;
    }

    tracing::info!(log_index, "--- voters {{1,2}}, removing node 0, are refused");
    {
        let err = leader.change_membership([1, 2], false).await.unwrap_err();

        let left_out = LeaderNotInJointMembership {
            leader_id: 0,
            joint: vec![btreeset! {1,2,3}, btreeset! {1,2}],
        };
        let want = ClientWriteError::ChangeMembershipError(ChangeMembershipError::LeaderNotInJointMembership(left_out));
        assert_eq!(RaftError::APIError(want), err);

        let metrics = leader.metrics().borrow_watched().clone();
        assert_eq!(Some(log_index), metrics.last_log_index);
    }

    tracing::info!(
        log_index,
        "--- voters {{1,2}}, retaining node 0 as a learner, are accepted"
    );
    {
        let resp = leader.change_membership([1, 2], true).await?;
        log_index += 2;

        assert_eq!(log_index, resp.log_id.index);
        let want = Membership::new_with_defaults(vec![btreeset! {1,2}], btreeset! {0,3});
        assert_eq!(Some(want), resp.membership);
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
