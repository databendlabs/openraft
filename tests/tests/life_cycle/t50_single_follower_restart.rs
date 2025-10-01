use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use openraft::Config;
use openraft::RaftLogReader;
use openraft::ServerState;
use openraft::Vote;
use openraft::storage::RaftLogStorage;
use openraft::vote::RaftLeaderId;
use openraft::vote::RaftLeaderIdExt;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// Brings up a cluster of 1 node and restart it, when it is a follower.
///
/// The single follower should become leader very quickly. Because it does not need to wait for an
/// active leader to replicate to it.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn single_follower_restart() -> anyhow::Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            election_timeout_min: 3_000,
            election_timeout_max: 4_000,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- bring up cluster of 1 node");
    let mut log_index = router.new_cluster(btreeset! {0}, btreeset! {}).await?;

    tracing::info!(log_index, "--- write to 1 log");
    {
        router.client_request_many(0, "foo", 1).await?;
        log_index += 1;
    }

    tracing::info!(log_index, "--- stop and restart node-0");
    {
        let (node, mut sto, sm) = router.remove_node(0).unwrap();
        node.shutdown().await?;
        let v = sto.read_vote().await?.unwrap_or_default();

        // Set a non-committed vote so that the node restarts as a follower.
        sto.save_vote(&Vote::new(v.leader_id.term() + 1, v.leader_id.to_node_id().unwrap())).await?;

        tracing::info!(log_index, "--- restart node-0");

        router.new_raft_node_with_sto(0, sto, sm).await;
        router
            .wait(&0, Some(Duration::from_millis(1_000)))
            .state(ServerState::Leader, "single node restarted an became leader quickly")
            .await?;

        // Leader blank log
        log_index += 1;
    }

    tracing::info!(log_index, "--- write to 1 log after restart");
    {
        router.client_request_many(0, "foo", 1).await?;
        log_index += 1;

        router.wait(&0, timeout()).applied_index(Some(log_index), "node-0 works").await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
