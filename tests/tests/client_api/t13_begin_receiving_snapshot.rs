use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use openraft::Config;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn begin_receiving_snapshot() -> anyhow::Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            enable_elect: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    tracing::info!(log_index, "--- isolate node 2 so that it can receive snapshot");
    router.set_unreachable(2, true);

    tracing::info!(log_index, "--- write to make node-0,1 have more logs");
    {
        log_index += router.client_request_many(0, "foo", 3).await?;
        router.wait(&0, timeout()).applied_index(Some(log_index), "write more log").await?;
        router.wait(&1, timeout()).applied_index(Some(log_index), "write more log").await?;
    }

    tracing::info!(log_index, "--- got a snapshot data");
    {
        let n1 = router.get_raft_handle(&1)?;
        let _resp = n1.begin_receiving_snapshot().await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
