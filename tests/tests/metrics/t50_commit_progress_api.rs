use std::sync::Arc;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// Test commit progress API: get() and wait_until_ge()
///
/// What does this test do?
///
/// - Creates a single-node cluster
/// - Gets `watch_commit_progress()` handle
/// - Verifies initial state with `get()`
/// - Writes client requests to advance the committed log id
/// - Uses `wait_until_ge()` with a concrete target value
/// - Verifies `get()` returns the same value as `wait_until_ge()`
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn commit_progress_api() -> Result<()> {
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

    tracing::info!(log_index, "--- get commit progress watcher");
    let n0 = router.get_raft_handle(&0)?;
    let progress = n0.watch_commit_progress();

    tracing::info!(log_index, "--- verify initial commit progress with get()");
    let got = progress.get();
    let want = Some(log_id(1, 0, log_index));
    assert_eq!(got, want);

    tracing::info!(log_index, "--- spawn task to wait for future commit progress");
    let target_index = log_index + 5;
    let target = Some(log_id(1, 0, target_index));

    let n0_clone = router.get_raft_handle(&0)?;
    let handle = tokio::spawn(async move {
        let mut progress = n0_clone.watch_commit_progress();
        progress.wait_until_ge(&target).await
    });

    tracing::info!(log_index, "--- send client requests to trigger wait_until_ge return");
    log_index += router.client_request_many(0, "foo", 5).await?;

    tracing::info!(log_index, "--- verify wait_until_ge returns after logs are committed");
    let got_wait = handle.await??;
    let got_get = progress.get();

    let want = Some(log_id(1, 0, log_index));
    assert_eq!(got_wait, want);
    assert_eq!(got_get, want);

    Ok(())
}
