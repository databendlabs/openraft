use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::ServerState;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// A restarted unreachable node should recover correctly, and catch up with the leader:
/// - Setup cluster {0,1,2}, Shutdown leader node-0;
/// - Elect node-1, write some logs;
/// - Restart node-0, it should receive append-entries from node-1 and catch up.
#[async_entry::test(worker_threads = 4, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn append_entries_backoff_rejoin() -> Result<()> {
    let config = Arc::new(
        Config {
            election_timeout_min: 100,
            election_timeout_max: 200,
            enable_elect: false,
            // Disable heartbeat to make the problem clear.
            enable_heartbeat: true,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    let n = 10;

    tracing::info!(log_index, "--- set node 0 to unreachable and remove it");
    router.set_unreachable(0, true);

    let (_, ls0, sm0) = router.remove_node(0).unwrap();
    let n1 = router.get_raft_handle(&1)?;

    tracing::info!(log_index, "--- elect node-1");
    {
        // Timeout leader lease otherwise vote-request will be rejected by node-2
        tokio::time::sleep(Duration::from_millis(1_000)).await;

        n1.trigger().elect().await?;
        n1.wait(timeout()).state(ServerState::Leader, "node-1 elect").await?;
    }

    tracing::info!(log_index, "--- write {} entries to node-1", n);
    {
        log_index += router.client_request_many(1, "1", n as usize).await?;
        n1.wait(timeout())
            .applied_index_at_least(Some(log_index), format!("node-1 commit {} writes", n))
            .await?;
    }

    tracing::info!(log_index, "--- restart node-0, check replication");
    {
        router.new_raft_node_with_sto(0, ls0, sm0).await;
        router.set_unreachable(0, false);

        router
            .wait(&0, timeout())
            .applied_index_at_least(Some(log_index), format!("node-0 commit {} writes", n))
            .await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
