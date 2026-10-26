use std::time::Duration;

use openraft::Config;

use crate::fixtures::Node;
use crate::fixtures::RaftRouter;
use crate::fixtures::expect_timeout;
use crate::fixtures::ut_harness;

/// A follower that restarted should not interrupt a stable cluster by a too quick election.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn follower_restart_does_not_interrupt() -> anyhow::Result<()> {
    let config = Config {
        enable_heartbeat: false,
        election_timeout_min: 3_000,
        election_timeout_max: 4_000,
        ..Default::default()
    };

    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- bring up cluster of 3 nodes");
    let log_index = router.new_cluster([0, 1, 2], []).await?;
    let _ = log_index;

    tracing::info!(log_index, "--- stop and restart follower nodes 1,2");
    {
        // Stop followers first or the follower may start re-electing.

        let m = router.get_metrics(&0)?;
        let term = m.current_term;

        let Node {
            log_store: sto2,
            state_machine: sm2,
            ..
        } = router.stop_node(2).await?;

        let Node {
            log_store: sto1,
            state_machine: sm1,
            ..
        } = router.stop_node(1).await?;

        router.stop_node(0).await?;

        tracing::info!(log_index, "--- restart node 1,2");

        router.new_raft_node_with_sto(1, sto1, sm1).await?;
        router.new_raft_node_with_sto(2, sto2, sm2).await?;
        let res = router
            .wait_with_timeout(&1, Duration::from_millis(1_000))
            .metrics(|x| x.current_term > term, "node increase term to start election")
            .await;

        expect_timeout(res)?;

        router
            .wait_with_timeout(&1, Duration::from_millis(9_000))
            .metrics(
                |x| x.current_term > term,
                concat!(
                    "node increase term to start election after a election timeout.",
                    "When it starts up, it set the last-update time of the vote to `now`, ",
                    "which make it as if a just renewed vote(has a full leader lease)."
                ),
            )
            .await?;
    }

    Ok(())
}
