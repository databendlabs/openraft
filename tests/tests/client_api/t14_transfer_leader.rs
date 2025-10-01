use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use openraft::Config;
use openraft::ServerState;
use openraft::raft::TransferLeaderRequest;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// Test handling of transfer leader request.
///
/// Call [`handle_transfer_leader`](openraft::raft::Raft::handle_transfer_leader) on every
/// non-leader node to force establish a new leader.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn transfer_leader() -> anyhow::Result<()> {
    let config = Arc::new(
        Config {
            election_timeout_min: 150,
            election_timeout_max: 300,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let _log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    let n0 = router.get_raft_handle(&0)?;
    let n1 = router.get_raft_handle(&1)?;
    let n2 = router.get_raft_handle(&2)?;

    let metrics = n0.metrics().borrow().clone();
    let leader_vote = metrics.vote;
    let last_log_id = metrics.last_applied;

    let req = TransferLeaderRequest::new(leader_vote, 2, last_log_id);

    tracing::info!("--- transfer Leader from 0 to 2");
    {
        n1.handle_transfer_leader(req.clone()).await?;
        n2.handle_transfer_leader(req.clone()).await?;

        n2.wait(timeout()).state(ServerState::Leader, "node-2 become leader").await?;
        n0.wait(timeout()).state(ServerState::Follower, "node-0 become follower").await?;
    }

    tracing::info!("--- cannot transfer Leader from 2 to 0 with an old vote");
    {
        let req = TransferLeaderRequest::new(leader_vote, 0, last_log_id);

        n0.handle_transfer_leader(req.clone()).await?;
        n1.handle_transfer_leader(req.clone()).await?;

        let n0_res = n0
            .wait(Some(Duration::from_millis(1_000)))
            .state(ServerState::Leader, "node-0 cannot become leader with old leader vote")
            .await;

        assert!(n0_res.is_err());
    }

    Ok(())
}

/// Test trigger transfer leader on the Leader.
///
/// Call [`trigger().transfer_leader()`](openraft::raft::trigger::Trigger::transfer_leader) on the
/// Leader node to force establish a new leader.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn trigger_transfer_leader() -> anyhow::Result<()> {
    let config = Arc::new(
        Config {
            election_timeout_min: 150,
            election_timeout_max: 300,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let _log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    let n0 = router.get_raft_handle(&0)?;
    let n1 = router.get_raft_handle(&1)?;
    let n2 = router.get_raft_handle(&2)?;

    tracing::info!("--- trigger transfer Leader from 0 to 2");
    {
        n0.trigger().transfer_leader(2).await?;

        n2.wait(timeout()).state(ServerState::Leader, "node-2 become leader").await?;
        n0.wait(timeout()).state(ServerState::Follower, "node-0 become follower").await?;
        n1.wait(timeout()).state(ServerState::Follower, "node-1 become follower").await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(500))
}
