use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::storage::RaftLogStorage;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// Metric `purged` should be the last purged log id.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn metrics_purged() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            max_in_snapshot_log_to_keep: 0,
            purge_batch_size: 1,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initialize cluster");
    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    let n = 10;
    tracing::info!(log_index, "--- write {} logs", n);
    log_index += router.client_request_many(0, "foo", n).await?;

    tracing::info!(log_index, "--- trigger snapshot");
    {
        let n0 = router.get_raft_handle(&0)?;
        n0.trigger().snapshot().await?;
        n0.wait(timeout()).snapshot(log_id(1, 0, log_index), "build snapshot").await?;

        tracing::info!(log_index, "--- metrics reports purged log id");
        n0.wait(timeout())
            .metrics(
                |m| m.purged == Some(log_id(1, 0, log_index)),
                "purged is reported to metrics",
            )
            .await?;

        tracing::info!(log_index, "--- check storage at once to ensure purged log is removed");
        let (mut sto0, _sm0) = router.get_storage_handle(&0)?;
        let state = sto0.get_log_state().await?;
        assert_eq!(state.last_purged_log_id, Some(log_id(1, 0, log_index)));
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
