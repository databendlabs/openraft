use std::time::Duration;

use anyhow::Result;
use openraft::Config;
use openraft::ServerState;
use openraft::async_runtime::WatchReceiver;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// A candidate that cannot win retries its election with the internal ticker disabled.
///
/// Node 1 is isolated, so its first election fails. Its retry at the next election timeout starts
/// from `ElectionDeadlineReached`, because no tick arrives.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn candidate_retries_without_the_internal_ticker() -> Result<()> {
    let config = Config {
        enable_tick: false,
        enable_pre_vote: Some(false),
        heartbeat_interval: 100,
        election_timeout_min: 500,
        election_timeout_max: 501,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- create cluster of 0,1,2; node 0 becomes leader");
    let log_index = router.new_cluster([0, 1, 2], []).await?;
    let term = router.get_raft_handle(&1)?.metrics().borrow_watched().current_term;

    tracing::info!(
        log_index,
        term,
        "--- disable elections on node 2 and isolate node 1, so that every campaign of node 1 fails"
    );
    {
        router.get_raft_handle(&2)?.runtime_config().elect(false);
        router.set_network_error(1, true);
    }

    tracing::info!(
        log_index,
        term,
        "--- node 1 campaigns at term+1, then retries at term+2 without a tick"
    );
    {
        let metrics = router.wait(&1).metrics(|m| m.current_term >= term + 2, "node 1 retries its election").await?;
        assert_eq!(ServerState::Candidate, metrics.state);
    }

    Ok(())
}

/// A leader whose leadership transfer times out campaigns with the internal ticker disabled.
///
/// The transfer target, node 3, does not exist, and elections are disabled on nodes 1 and 2, so
/// only node 0 can end the transfer. Its election starts from `ElectionDeadlineReached`, because no
/// tick arrives.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn leader_campaigns_after_failed_transfer_without_the_internal_ticker() -> Result<()> {
    let config = Config {
        enable_tick: false,
        heartbeat_interval: 100,
        election_timeout_min: 500,
        election_timeout_max: 501,
        ..Default::default()
    };
    let election_timeout_max = Duration::from_millis(config.election_timeout_max);
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- create cluster of 0,1,2; node 0 becomes leader");
    let log_index = router.new_cluster([0, 1, 2], []).await?;
    let n0 = router.get_raft_handle(&0)?;
    let vote_before = n0.metrics().borrow_watched().vote;

    tracing::info!(
        log_index,
        vote = %vote_before,
        "--- disable elections on nodes 1 and 2, then transfer leadership to node 3"
    );
    {
        for id in [1, 2] {
            router.get_raft_handle(&id)?.runtime_config().elect(false);
        }
        n0.trigger().transfer_leader(3).await?;
    }

    tracing::info!(
        log_index,
        vote = %vote_before,
        "--- node 0 steps down once the transfer times out and leads a new term"
    );
    {
        n0.wait(Some(election_timeout_max * 5))
            .metrics(
                |m| m.state == ServerState::Leader && m.vote.is_committed() && m.vote > vote_before,
                "node 0 leads a new term",
            )
            .await?;
    }

    Ok(())
}
