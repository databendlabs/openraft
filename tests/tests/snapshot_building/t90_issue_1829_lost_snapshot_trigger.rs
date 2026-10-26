use anyhow::Result;
use openraft::Config;
use openraft::SnapshotPolicy;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::BlockOperation;
use openraft_memstore::ClientRequest;
use openraft_memstore::IntoMemClientRequest;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::timeout;
use crate::fixtures::ut_harness;

/// A `SnapshotPolicy::LogsSinceLast` trigger that races an in-flight build must not be lost.
///
/// Reproduces <https://github.com/databendlabs/openraft/issues/1829>.
///
/// A policy-fired trigger that arrives while a previous snapshot build is still running is dropped
/// by `SnapshotHandler::trigger_snapshot` (it returns `false` when `building_snapshot` is set), but
/// `RaftCore::trigger_routine_actions` advances `snapshot_tried_at` anyway. Nothing re-checks the
/// policy when the in-flight build completes, and the phantom `snapshot_tried_at` then suppresses
/// `should_snapshot` (which bases its decision on `max(snapshot_tried_at, snapshot_last)`). On a
/// quiescent node the snapshot stays pinned arbitrarily far behind `committed`.
///
/// The scenario, on a single node with `LogsSinceLast(10)`:
///
/// - Bring `committed == applied == 9` so the policy fires and starts building snapshot `A`. The
///   state machine holds the build open (`BlockOperation::BuildSnapshot`), which keeps
///   `building_snapshot == true` and, because the held read lock blocks apply, pins `applied` at 9.
/// - Fire-and-forget writes advance `committed` to 23 while `applied` stays pinned. The policy
///   fires a second time, but the trigger is dropped since build `A` is still in flight -- yet
///   `snapshot_tried_at` advances to it.
/// - Build `A` completes at its captured position 9.
///
/// Expected (and asserted): once build `A` completes, the policy re-arms and the snapshot converges
/// to `committed == 23` without any further writes. Before the fix the snapshot stays stuck at 9,
/// so the final wait times out.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn issue_1829_lost_snapshot_trigger() -> Result<()> {
    /// `committed` index at which the policy first fires: `base(None).next_index()(0) + threshold`.
    const FIRST: u64 = 9;
    /// `committed` index after the fire-and-forget burst; the second (dropped) fire happens on the
    /// way here, and 23 stays below the `snapshot_tried_at + threshold` re-fire point.
    const FINAL: u64 = 23;

    let config = Config {
        snapshot_policy: SnapshotPolicy::LogsSinceLast(10),
        enable_heartbeat: false,
        enable_tick: false,
        ..Default::default()
    };

    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initializing single-node cluster");
    let log_index = router.new_cluster([0], []).await?;
    let n0 = router.get_raft_handle(&0)?;

    tracing::info!(log_index, "--- hold the state machine's snapshot build open");
    let gate = {
        let (_, sm) = router.get_storage_handle(&0)?;
        sm.block.pause(BlockOperation::BuildSnapshot)
    };

    tracing::info!(
        log_index,
        "--- write up to committed == applied == {}, firing the policy once",
        FIRST
    );
    {
        let count = (FIRST - log_index) as usize;
        router.client_request_many(0, "cli", count).await?;
        router.wait(&0).applied_index(Some(FIRST), "applied up to the first threshold").await?;
        TypeConfig::timeout(timeout().unwrap(), gate.entered()).await?;
    }

    tracing::info!(
        "--- fire-and-forget writes to advance committed to {} while apply is pinned",
        FINAL
    );
    {
        for i in 0..(FINAL - FIRST) {
            n0.client_write_ff(ClientRequest::make_request("ff", i), None).await?;
        }
    }

    tracing::info!(
        "--- confirm build A is in flight: committed reached {} but apply is pinned",
        FINAL
    );
    let m = router.wait(&0).committed_index(Some(FINAL), "fire-and-forget writes committed").await?;
    let last_applied = m.last_applied;
    assert_eq!(Some(log_id(1, 0, FIRST)), last_applied);

    tracing::info!(
        "--- release build A: it completes at 9, then the pinned applies catch up to {}",
        FINAL
    );
    gate.release();
    router.wait(&0).applied_index(Some(FINAL), "build A released; applies catch up").await?;

    tracing::info!("--- snapshot must re-arm and converge to committed == {}", FINAL);
    router
        .wait(&0)
        .snapshot(
            log_id(1, 0, FINAL),
            "snapshot converges after the in-flight build completes",
        )
        .await?;

    Ok(())
}
