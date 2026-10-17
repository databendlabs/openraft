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
use openraft_memstore::ClientRequest;
use openraft_memstore::ClientResponse;
use openraft_memstore::IntoMemClientRequest;

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

/// A removed leader continues accepting writes and every membership API until it steps down.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn removed_leader_accepts_writes_and_membership_changes() -> Result<()> {
    let mut router = RaftRouter::new(config()?);

    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {3}).await?;
    let leader = router.get_raft_handle(&0)?;

    tracing::info!(log_index, "--- node 0 removes itself and stays the leader");
    {
        let response = leader.change_membership([1, 2], false).await?;
        log_index = response.log_id.index;
        let expected = Membership::new_with_defaults(vec![btreeset! {1,2}], btreeset! {3});
        assert_eq!(Some(expected), response.membership);

        leader.wait(timeout()).applied_index(Some(log_index), "the removal is applied").await?;
        let metrics = leader.metrics().borrow_watched().clone();
        assert_eq!(ServerState::Leader, metrics.state);
    }

    tracing::info!(log_index, "--- write through node 0 after it leaves the membership");
    {
        let request = ClientRequest::make_request("removed-leader", 0);
        let response = leader.client_write(request).await?;
        log_index += 1;
        assert_eq!(log_index, response.log_id.index);
        assert_eq!(ClientResponse(None), response.data);
    }

    tracing::info!(log_index, "--- remove the leftover learner through node 0");
    {
        let response = leader.change_membership(ChangeMembers::RemoveNodes(btreeset! {3}), false).await?;
        log_index = response.log_id.index;
        let expected = Membership::new_with_defaults(vec![btreeset! {1,2}], []);
        assert_eq!(Some(expected), response.membership);
    }

    tracing::info!(log_index, "--- add a learner through node 0");
    {
        router.new_raft_node(4).await;
        let response = leader.add_learner(4, (), false).await?;
        log_index += 1;
        assert_eq!(log_index, response.log_id.index);
        let expected = Membership::new_with_defaults(vec![btreeset! {1,2}], btreeset! {4});
        assert_eq!(Some(expected), response.membership);
    }

    tracing::info!(log_index, "--- append a membership through node 0 to remove learner 4");
    {
        let proposed = Membership::new_with_defaults(vec![btreeset! {1,2}], []);
        let response = leader.append_membership(proposed.clone(), EntryPayload::Blank, []).await?;
        log_index += 1;
        assert_eq!(log_index, response.log_id.index);
        assert_eq!(Some(proposed), response.membership);
    }

    tracing::info!(
        log_index,
        "--- another write preserves the earlier value and reaches all voters"
    );
    {
        let request = ClientRequest::make_request("removed-leader", 1);
        let response = leader.client_write(request).await?;
        log_index += 1;
        assert_eq!(log_index, response.log_id.index);
        let previous = "request-0".to_string();
        let expected = ClientResponse(Some(previous));
        assert_eq!(expected, response.data);

        for node_id in [0, 1, 2] {
            router.wait(&node_id, timeout()).applied_index(Some(log_index), "writes are applied").await?;
        }

        let metrics = leader.metrics().borrow_watched().clone();
        assert_eq!(Some(log_index), metrics.last_log_index);
        assert_eq!(ServerState::Leader, metrics.state);
        let expected = Membership::new_with_defaults(vec![btreeset! {1,2}], []);
        let actual = metrics.membership_config.membership();
        assert_eq!(&expected, actual);
    }

    Ok(())
}

/// From the joint membership `[{0,1,2}, {1,2,3}]`, leader 0 is asked for voters `{1,2}`.
///
/// The change completes and node 0 can still write, whether removed or retained as a learner.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn change_from_joint_can_leave_out_the_leader() -> Result<()> {
    for retain in [false, true] {
        let mut router = RaftRouter::new(config()?);

        let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {3}).await?;
        let leader = router.get_raft_handle(&0)?;

        tracing::info!(
            log_index,
            retain,
            "--- leave the cluster in a joint membership, as an interrupted change would"
        );
        {
            let joint = Membership::new_with_defaults(vec![btreeset! {0,1,2}, btreeset! {1,2,3}], []);
            leader.append_membership(joint, EntryPayload::Blank, []).await?;
            log_index += 1;

            leader.wait(timeout()).applied_index(Some(log_index), "the joint membership is applied").await?;
        }

        tracing::info!(log_index, retain, "--- node 0 completes the change to voters {{1,2}}");
        {
            let response = leader.change_membership([1, 2], retain).await?;
            log_index = response.log_id.index;

            let learners = if retain {
                btreeset! {0,3}
            } else {
                btreeset! {}
            };
            let expected = Membership::new_with_defaults(vec![btreeset! {1,2}], learners);
            assert_eq!(Some(expected), response.membership);
        }

        tracing::info!(
            log_index,
            retain,
            "--- node 0 still accepts writes after completing the change"
        );
        {
            let request = ClientRequest::make_request("after-joint", 0);
            let response = leader.client_write(request).await?;
            log_index += 1;
            assert_eq!(log_index, response.log_id.index);
            assert_eq!(ClientResponse(None), response.data);

            for node_id in [0, 1, 2] {
                router.wait(&node_id, timeout()).applied_index(Some(log_index), "write after joint change").await?;
            }

            let metrics = leader.metrics().borrow_watched().clone();
            assert_eq!(ServerState::Leader, metrics.state);
        }
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
