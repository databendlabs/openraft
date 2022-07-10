use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::RaftLogReader;
use tokio::time::sleep;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Logs should be deleted by raft after applying them, on leader and learner.
///
/// - assert logs are deleted on leader after applying them.
/// - assert logs are deleted on replication target after installing a snapshot.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn clean_applied_logs() -> Result<()> {
    // Setup test dependencies.
    let config = Arc::new(
        Config {
            max_applied_log_to_keep: 2,
            purge_batch_size: 1,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let mut log_index = router.new_nodes_from_single(btreeset! {0}, btreeset! {1}).await?;

    let count = (10 - log_index) as usize;
    for idx in 0..count {
        router.client_request(0, "0", idx as u64).await?;
        // raft commit at once with a single leader cluster.
        // If we send too fast, logs are removed before forwarding to learner.
        // Then it triggers snapshot replication, which is not expected.
        sleep(Duration::from_millis(50)).await;
    }
    log_index = 10;

    router.wait_for_log(&btreeset! {0,1}, Some(log_index), timeout(), "write upto 10 logs").await?;

    tracing::info!("--- logs before max_applied_log_to_keep should be cleaned");
    {
        for node_id in 0..1 {
            let mut sto = router.get_storage_handle(&node_id)?;
            let logs = sto.get_log_entries(..).await?;
            assert_eq!(2, logs.len(), "node {} should have only {} logs", node_id, 2);
        }
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(5000))
}
