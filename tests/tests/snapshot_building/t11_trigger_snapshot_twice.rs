use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use openraft::CommittedLeaderId;
use openraft::Config;
use openraft::LogId;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Two back-to-back `Raft::trigger().snapshot()` calls targeting the same
/// `last_applied` must not panic on the monotonic-advancement invariant of
/// `IOState::update_snapshot`.
///
/// Regression for the path where:
///
/// 1. First trigger builds a snapshot at log id `X` and completes. The engine's
///    `building_snapshot` flag is cleared and `IOState::snapshot` is updated to `X`.
/// 2. A second trigger arrives before any new entry is applied. A new
///    `BuildSnapshot` command is queued at the same `last_applied == X`.
/// 3. When that second build completes, `SnapshotHandler::update_snapshot`
///    correctly returns `false` (equal log id, no advancement), but `raft_core`
///    previously called `IOState::update_snapshot(X)` unconditionally, tripping
///    `debug_assert!(log_id > self.snapshot)` in debug builds.
///
/// With the fix, `raft_core` honors the `bool` returned by
/// `Engine::finish_building_snapshot` and only updates the io-state snapshot
/// cursor when the engine actually advanced.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn trigger_snapshot_twice_at_same_last_applied() -> anyhow::Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing single-node cluster");
    let log_index = router.new_cluster(btreeset! {0}, btreeset! {}).await?;

    let n0 = router.get_raft_handle(&0)?;

    let snap_at = LogId::new(CommittedLeaderId::new(1, 0), log_index);

    tracing::info!(log_index, "--- first trigger().snapshot(): build at {}", snap_at);
    n0.trigger().snapshot().await?;
    router.wait(&0, timeout()).snapshot(snap_at, "first snapshot built").await?;

    tracing::info!(
        log_index,
        "--- second trigger().snapshot() at the same last_applied: must not panic"
    );
    n0.trigger().snapshot().await?;

    // The second build produces the same snapshot meta, so the snapshot metric
    // never changes. Wait a generous window for the second build to complete —
    // if the regression is present, `IOState::update_snapshot` panics in the
    // raft core task during this window.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Prove the raft core task is still alive: issue a client write and wait
    // for it to be applied. A panicked core would leave this hanging until the
    // test timeout.
    tracing::info!(log_index, "--- client write after duplicate snapshot: proves raft core survived");
    router.client_request_many(0, "after_dup_snap", 1).await?;
    let log_index = log_index + 1;
    router
        .wait(&0, timeout())
        .applied_index(Some(log_index), "client write applied after duplicate snapshot")
        .await?;

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
