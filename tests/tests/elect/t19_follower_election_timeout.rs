use std::time::Duration;

use anyhow::Result;
use openraft::Config;
use openraft::ServerState;
use openraft::async_runtime::WatchReceiver;

use crate::fixtures::RaftRouter;
use crate::fixtures::expect_timeout;
use crate::fixtures::ut_harness;

/// A follower whose leader stops elects at its election timeout.
///
/// The internal ticker no longer checks a follower's election, so this election starts from the
/// `ElectionDeadlineReached` notification sent when the election may be due.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn follower_elects_through_its_election_timeout() -> Result<()> {
    let config = Config::default();
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- create cluster of 0,1,2; node 0 becomes leader");
    let log_index = router.new_cluster([0, 1, 2], []).await?;

    tracing::info!(
        log_index,
        "--- disable elections on node 2, so that node 1 is the one to elect"
    );
    router.get_raft_handle(&2)?.runtime_config().elect(false);

    tracing::info!(
        log_index,
        "--- isolate the leader; node 1 must elect itself once its lease expires"
    );
    {
        router.set_network_error(0, true);
        router.wait(&1).state(ServerState::Leader, "node 1 elected").await?;
    }

    Ok(())
}

/// The runtime `elect` switch stops and restarts a follower's automatic election.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn runtime_elect_switch_controls_the_follower_election_timeout() -> Result<()> {
    let config = Config::default();
    let election_timeout_max = Duration::from_millis(config.election_timeout_max);
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- create cluster of 0,1,2; node 0 becomes leader");
    let log_index = router.new_cluster([0, 1, 2], []).await?;
    let term = router.get_raft_handle(&1)?.metrics().borrow_watched().current_term;

    tracing::info!(
        log_index,
        term,
        "--- disable elections on the followers and isolate the leader"
    );
    {
        for id in [1, 2] {
            router.get_raft_handle(&id)?.runtime_config().elect(false);
        }
        router.set_network_error(0, true);
    }

    tracing::info!(
        log_index,
        term,
        "--- several election timeouts pass without an election on node 1"
    );
    {
        let res = router
            .wait_with_timeout(&1, election_timeout_max * 5)
            .metrics(|m| m.current_term > term, "node 1 starts an election")
            .await;
        expect_timeout(res)?;
    }

    tracing::info!(
        log_index,
        term,
        "--- re-enable elections on node 1; it must elect itself"
    );
    {
        router.get_raft_handle(&1)?.runtime_config().elect(true);
        router.wait(&1).state(ServerState::Leader, "node 1 elected").await?;
    }

    Ok(())
}
