use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use openraft::Config;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Brings up a cluster of 1 node and restart it.
///
/// Assert that `RaftCore.engine.state.server_state` and `RaftCore.leader_data` are consistent:
/// `server_state == Leader && leader_data.is_some() || server_state != Leader &&
/// leader_data.is_none()`.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn single_restart() -> anyhow::Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
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

    tracing::info!(log_index, "--- stop and restart node 0");
    {
        let (node, sto, sm) = router.remove_node(0).unwrap();
        node.shutdown().await?;

        tracing::info!(log_index, "--- restart node 0");

        router.new_raft_node_with_sto(0, sto, sm).await;
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
    Some(Duration::from_millis(1000))
}
