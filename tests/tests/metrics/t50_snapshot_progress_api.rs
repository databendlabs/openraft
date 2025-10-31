use std::sync::Arc;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// Test snapshot progress API: get() and wait_until_ge()
///
/// What does this test do?
///
/// - Creates a single-node cluster
/// - Gets `watch_snapshot_progress()` handle
/// - Verifies initial state with `get()`
/// - Triggers snapshot to advance the snapshot log id
/// - Uses `wait_until_ge()` with a concrete target value
/// - Verifies `get()` returns the same value as `wait_until_ge()`
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn snapshot_progress_api() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let mut log_index = router.new_cluster(btreeset! {0}, btreeset! {}).await?;

    tracing::info!(log_index, "--- get snapshot progress watcher");
    let n0 = router.get_raft_handle(&0)?;
    let progress = n0.watch_snapshot_progress();

    tracing::info!(log_index, "--- verify initial snapshot progress with get()");
    let got = progress.get();
    assert_eq!(got, None);

    tracing::info!(log_index, "--- write some logs");
    log_index += router.client_request_many(0, "foo", 5).await?;

    tracing::info!(log_index, "--- spawn task to wait for future snapshot progress");
    let target = Some(log_id(1, 0, log_index));

    let n0_clone = router.get_raft_handle(&0)?;
    let handle = tokio::spawn(async move {
        let mut progress = n0_clone.watch_snapshot_progress();
        progress.wait_until_ge(&target).await
    });

    tracing::info!(log_index, "--- trigger snapshot to update snapshot progress");
    n0.trigger().snapshot().await?;

    tracing::info!(log_index, "--- verify wait_until_ge returns after snapshot is built");
    let got_wait = handle.await??;
    let got_get = progress.get();

    let want = Some(log_id(1, 0, log_index));
    assert_eq!(got_wait, want);
    assert_eq!(got_get, want);

    Ok(())
}
