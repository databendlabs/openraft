use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::testing::log_id;
use openraft::Config;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// When transferring snapshot to unreachable node, it should not block for ever.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn snapshot_to_unreachable_node_should_not_block() -> Result<()> {
    let config = Arc::new(
        Config {
            purge_batch_size: 1,
            max_in_snapshot_log_to_keep: 0,
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let mut log_index = router.new_cluster(btreeset! {0,1}, btreeset! {2}).await?;

    tracing::info!(log_index, "--- isolate replication 0 -> 2");
    router.set_network_error(2, true);

    let n = 10;
    tracing::info!(log_index, "--- write {} logs", n);
    {
        log_index += router.client_request_many(0, "0", n).await?;
        router.wait(&0, timeout()).applied_index(Some(log_index), format!("{} writes", n)).await?;
    }

    let n0 = router.get_raft_handle(&0)?;

    tracing::info!(log_index, "--- build a snapshot");
    {
        n0.trigger().snapshot().await?;

        n0.wait(timeout()).snapshot(log_id(1, 0, log_index), "snapshot").await?;
        n0.wait(timeout()).purged(Some(log_id(1, 0, log_index)), "logs in snapshot are purged").await?;
    }

    tracing::info!(
        log_index,
        "--- change membership to {{0}}, replication should be closed and re-spawned, snapshot streaming should stop at once"
    );
    {
        n0.change_membership([0], true).await?;
        n0.wait(timeout()).voter_ids([0], "change membership to {{0}}").await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
