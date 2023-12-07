use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use futures::StreamExt;
use maplit::btreeset;
use openraft::Config;
use openraft::SnapshotPolicy;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Compaction test.
///
/// What does this test do?
///
/// - build a stable single node cluster.
/// - send enough requests to the node that log compaction will be triggered.
/// - add new nodes and assert that they receive the snapshot.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn snapshot_policy_never() -> Result<()> {
    let n_logs: u64 = 6000;
    let default_config = Config::default().snapshot_policy;
    let default_threshold = if let SnapshotPolicy::LogsSinceLast(n) = default_config {
        n
    } else {
        panic!("snapshot_policy must be never");
    };

    assert!(
        n_logs > default_threshold,
        "snapshot_threshold must be greater than default threshold"
    );

    // Setup test dependencies.
    let config = Arc::new(
        Config {
            snapshot_policy: SnapshotPolicy::Never,
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let mut log_index = router.new_cluster(btreeset! {0}, btreeset! {}).await?;

    let mut clients = futures::stream::FuturesUnordered::new();

    let n_clients = 20;
    for i in 0..n_clients {
        let per_client = n_logs / n_clients;
        let r = router.clone();
        clients.push(async move {
            let client_id = format!("{}", i);
            r.client_request_many(0, &client_id, per_client as usize).await
        });
        log_index += per_client;
    }

    while clients.next().await.is_some() {}

    tracing::info!(log_index, "--- log_index: {}", log_index);
    router
        .wait(&0, timeout())
        .applied_index(Some(log_index), format_args!("write log upto {}", log_index))
        .await?;

    let wait_snapshot_res = router
        .wait(&0, Some(Duration::from_millis(3_000)))
        .metrics(|m| m.snapshot.is_some(), "no snapshot will be built")
        .await;

    assert!(wait_snapshot_res.is_err(), "no snapshot should be built");

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
