use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use openraft::Config;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// Two back-to-back `Raft::trigger().snapshot()` calls targeting the same
/// `last_applied` must not panic the raft core task.
///
/// Regression for a bug fixed on release-0.9 (commits 561b0776 / 253c31c1):
/// the second `BuildSnapshot` completes at the same `last_log_id` as the
/// first, and `raft_core` previously called `IOState::update_snapshot(X)`
/// unconditionally — tripping `debug_assert!(log_id > self.snapshot)`.
///
/// On this branch the bug is avoided by construction:
/// `Engine::on_building_snapshot_done` uses `try_update_all` (a strict `<`
/// comparison that silently no-ops on equal values) and gates
/// `SnapshotHandler::update_snapshot` on its `bool` return. This test locks
/// that invariant in so a future refactor can't reintroduce the panic.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
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
    let snap_at = log_id(1, 0, log_index);

    tracing::info!(log_index, "--- first trigger().snapshot(): build at {}", snap_at);
    n0.trigger().snapshot().await?;
    router.wait(&0, timeout()).snapshot(snap_at, "first snapshot built").await?;

    tracing::info!(
        log_index,
        "--- second trigger().snapshot() at the same last_applied: must not panic"
    );
    n0.trigger().snapshot().await?;

    // The duplicate build must complete before we probe liveness. There is no
    // deterministic signal available for this:
    //
    // - `build_snapshot` in the SM worker spawns the build into its own task (`core/sm/worker.rs`), so
    //   a subsequent `Apply` command is not ordered after it — a client write can be applied before
    //   build #2 finishes.
    // - The `snapshot` metric is the last included log id; build #2 targets the same log id as build
    //   #1, so the metric never changes.
    // - `building_snapshot` is engine-internal and not surfaced via metrics.
    //
    // Sleep a generous window for build #2 to complete and raft_core to
    // process its `BuildSnapshotDone`. memstore builds are sub-millisecond
    // in practice, so this is conservative.
    TypeConfig::sleep(Duration::from_millis(500)).await;

    // Liveness probe: a client write proves the raft core task is still
    // servicing commands. Under a regression that re-introduced the strict
    // assert, raft_core would have panicked while processing the duplicate
    // `BuildSnapshotDone`, and this write would hang until the test timeout.
    router.client_request_many(0, "after_dup_snap", 1).await?;
    let log_index = log_index + 1;
    router
        .wait(&0, timeout())
        .applied_index(Some(log_index), "raft core survived duplicate snapshot")
        .await?;

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
