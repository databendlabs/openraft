use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use openraft::Config;
use openraft::ServerState;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Test error handling for `RaftNetworkFactory::connect()`.
///
/// - `connect()` that returns error should not panic a node.
/// - Minority un-connectable nodes do not affect the cluster.
/// - Majority un-connectable nodes break the raft cluster.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn network_connection_error() -> anyhow::Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            enable_elect: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    let log_index = router.new_nodes_from_single(btreeset! {0, 1, 2}, btreeset!()).await?;

    tracing::info!("--- failure to add unreachable learner to cluster");
    {
        router.new_raft_node(3);
        router.set_connectable(3, false);

        let res = router.add_learner(0, 3).await;
        assert!(res.is_err());

        router
            .wait_for_log(
                &btreeset! {0, 1, 2},
                Some(log_index),
                timeout(),
                "add learner fail, no logs",
            )
            .await?;
    }

    let n1 = router.get_raft_handle(&1)?;
    let n2 = router.get_raft_handle(&2)?;

    tracing::info!("--- block node-2, make it un-connectable");
    {
        router.set_connectable(2, false);
    }

    tracing::info!("--- since there are no heartbeat is sent, let node-1 to elect");
    {
        n1.trigger_elect().await?;
        n1.wait(timeout()).state(ServerState::Leader, "node-1 become leader").await?;

        tracing::info!("--- cluster should work with only a node-2 un-connectable");
        router.client_request_many(1, "client", 1).await?;
    }

    tracing::info!("--- set node-1 un-connectable too. let node-2 to elect. It should fail");
    {
        router.set_connectable(1, false);
        n2.trigger_elect().await?;

        let res = n2.wait(timeout()).state(ServerState::Leader, "node-2 can not be leader").await;
        assert!(res.is_err());
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1000))
}
