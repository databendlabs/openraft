use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::SnapshotMeta;
use openraft::SnapshotPolicy;
use openraft::storage::RaftStateMachine;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// Test that state machine can refuse to build a snapshot.
///
/// What does this test do?
///
/// - Build a stable single node cluster and apply some logs.
/// - Have the state machine refuse snapshot building by returning None from
///   `try_create_snapshot_builder()`.
/// - Trigger snapshot building and verify no snapshot is created and state is unchanged.
/// - Have the state machine allow snapshot building again.
/// - Trigger snapshot building and verify it succeeds with state updated.
///
/// Note: `allow_build_snapshot()` is a test helper to control the state machine's behavior.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn sm_can_refuse_snapshot_building() -> Result<()> {
    let config = Arc::new(
        Config {
            snapshot_policy: SnapshotPolicy::Never,
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let mut log_index = router.new_cluster(btreeset! {0}, btreeset! {}).await?;

    tracing::info!(log_index, "--- send some logs");
    log_index += router.client_request_many(0, "0", 10).await?;

    router.wait(&0, timeout()).applied_index(Some(log_index), "write logs").await?;

    tracing::info!(
        log_index,
        "--- disable snapshot building via allow_build_snapshot(false)"
    );
    {
        let (_, sm) = router.get_storage_handle(&0)?;
        sm.allow_build_snapshot(false);
    }

    tracing::info!(
        log_index,
        "--- trigger snapshot with building disabled, should return None"
    );
    {
        let n0 = router.get_raft_handle(&0)?;
        n0.trigger().snapshot().await?;

        // Wait a bit to ensure the snapshot building attempt completes
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Verify no snapshot was created in storage
        let (_, mut sm) = router.get_storage_handle(&0)?;
        let snapshot = sm.get_current_snapshot().await?;
        assert!(
            snapshot.is_none(),
            "snapshot should not be created when building is disabled"
        );

        // Verify RaftState.snapshot_meta is unchanged
        let state = n0.with_raft_state(|st| st.clone()).await?;
        assert_eq!(
            SnapshotMeta::default(),
            state.snapshot_meta,
            "snapshot_meta should remain default when building is disabled"
        );
        // TODO: verify io_snapshot_last_log_id is None; require public method to access it.
    }

    tracing::info!(
        log_index,
        "--- re-enable snapshot building via allow_build_snapshot(true)"
    );
    {
        let (_, sm) = router.get_storage_handle(&0)?;
        sm.allow_build_snapshot(true);
    }

    tracing::info!(log_index, "--- trigger snapshot with building enabled, should succeed");
    {
        let n0 = router.get_raft_handle(&0)?;
        n0.trigger().snapshot().await?;

        router.wait(&0, timeout()).snapshot(log_id(1, 0, log_index), "snapshot created").await?;
    }

    tracing::info!(log_index, "--- verify snapshot was created with correct state");
    {
        let n0 = router.get_raft_handle(&0)?;

        // Verify snapshot in storage
        let (_, mut sm) = router.get_storage_handle(&0)?;
        let snapshot = sm.get_current_snapshot().await?;
        assert!(snapshot.is_some(), "snapshot should be created");

        let snapshot = snapshot.unwrap();
        assert_eq!(
            snapshot.meta.last_log_id,
            Some(log_id(1, 0, log_index)),
            "snapshot should contain all applied logs"
        );

        // Verify RaftState.snapshot_meta is updated
        let state = n0.with_raft_state(|st| st.clone()).await?;
        assert_eq!(
            Some(log_id(1, 0, log_index)),
            state.snapshot_meta.last_log_id,
            "snapshot_meta should be updated"
        );
    }

    tracing::info!(log_index, "--- disable again and verify re-trigger still works");
    {
        let (_, mut sm) = router.get_storage_handle(&0)?;
        sm.allow_build_snapshot(false);

        let n0 = router.get_raft_handle(&0)?;
        n0.trigger().snapshot().await?;

        tokio::time::sleep(Duration::from_millis(200)).await;

        // The old snapshot should still be there in storage
        let snapshot = sm.get_current_snapshot().await?;
        assert!(snapshot.is_some(), "old snapshot should still exist");
        assert_eq!(
            snapshot.unwrap().meta.last_log_id,
            Some(log_id(1, 0, log_index)),
            "snapshot meta should be unchanged"
        );

        // Verify RaftState.snapshot_meta is unchanged
        let state = n0.with_raft_state(|st| st.clone()).await?;
        assert_eq!(
            Some(log_id(1, 0, log_index)),
            state.snapshot_meta.last_log_id,
            "snapshot_meta should remain unchanged"
        );
    }

    tracing::info!(log_index, "--- add more logs and enable building again");
    {
        log_index += router.client_request_many(0, "0", 3).await?;

        router.wait(&0, timeout()).applied_index(Some(log_index), "write final logs").await?;

        let (_, mut sm) = router.get_storage_handle(&0)?;
        sm.allow_build_snapshot(true);

        let n0 = router.get_raft_handle(&0)?;
        n0.trigger().snapshot().await?;

        router.wait(&0, timeout()).snapshot(log_id(1, 0, log_index), "final snapshot").await?;

        // Verify final snapshot in storage
        let snapshot = sm.get_current_snapshot().await?;
        assert_eq!(
            snapshot.unwrap().meta.last_log_id,
            Some(log_id(1, 0, log_index)),
            "new snapshot should contain all logs"
        );

        // Verify final RaftState.snapshot_meta is updated
        let state = n0.with_raft_state(|st| st.clone()).await?;
        assert_eq!(
            Some(log_id(1, 0, log_index)),
            state.snapshot_meta.last_log_id,
            "final snapshot_meta should be updated"
        );
    }

    Ok(())
}

/// Test that when state machine refuses snapshot building with LogsSinceLast policy,
/// OpenRaft waits for the next threshold before retrying.
///
/// What does this test do?
///
/// - Build a stable single node cluster with `SnapshotPolicy::LogsSinceLast(5)`.
/// - Apply some logs and have the state machine refuse snapshot building.
/// - Verify that OpenRaft doesn't repeatedly call `try_create_snapshot_builder()`.
/// - Apply more logs to reach the next threshold.
/// - Verify that OpenRaft retries after the threshold is reached.
/// - Allow snapshot building and verify it succeeds.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn sm_refuse_with_logs_since_last_policy() -> Result<()> {
    fn take_snapshot_builder_count(router: &RaftRouter, node: u64) -> Result<u64> {
        let (_, sm) = router.get_storage_handle(&node)?;
        Ok(sm.take_try_create_snapshot_builder_count())
    }

    let config = Arc::new(
        Config {
            snapshot_policy: SnapshotPolicy::LogsSinceLast(5),
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let mut log_index = router.new_cluster(btreeset! {0}, btreeset! {}).await?;

    tracing::info!(log_index, "--- disable snapshot building from the start");
    {
        let (_, sm) = router.get_storage_handle(&0)?;
        sm.allow_build_snapshot(false);
    }

    tracing::info!(
        log_index,
        "--- apply upto index=4 logs to trigger first snapshot attempt"
    );
    log_index += router.client_request_many(0, "0", 4 - log_index as usize).await?;
    router.wait(&0, timeout()).applied_index(Some(log_index), "applied upto index=4 logs").await?;

    tracing::info!(log_index, "--- applied upto index=4 logs");

    tokio::time::sleep(Duration::from_millis(300)).await;

    let first_count = take_snapshot_builder_count(&router, 0)?;

    tracing::info!(first_count, "--- first attempt count (should be >= 1)");
    assert!(first_count >= 1, "expected at least one attempt, got {}", first_count);

    tracing::info!(log_index, "--- apply 3 more logs (not enough to reach threshold)");
    log_index += router.client_request_many(0, "0", 3).await?;
    router.wait(&0, timeout()).applied_index(Some(log_index), "applied 3 more logs").await?;

    // Reset counter after applying logs to isolate the idle period
    take_snapshot_builder_count(&router, 0)?;

    // Apply a small write to trigger routine actions during the idle period
    tokio::time::sleep(Duration::from_millis(300)).await;
    log_index += router.client_request_many(0, "0", 1).await?;
    router.wait(&0, timeout()).applied_index(Some(log_index), "trigger check").await?;

    let second_count = take_snapshot_builder_count(&router, 0)?;

    tracing::info!(
        second_count,
        "--- second count after small write (should be 0, not retrying)"
    );
    assert_eq!(
        second_count, 0,
        "expected no retry before reaching threshold, got {}",
        second_count
    );

    tracing::info!(log_index, "--- apply 5 more logs to definitely reach threshold");
    log_index += router.client_request_many(0, "0", 5).await?;
    router.wait(&0, timeout()).applied_index(Some(log_index), "applied 5 more logs").await?;

    let third_count = take_snapshot_builder_count(&router, 0)?;
    tracing::info!(third_count, log_index, "third measurement");

    tracing::info!(
        third_count,
        log_index,
        "--- third count after reaching threshold (should be >= 1, retrying)"
    );
    assert!(
        third_count >= 1,
        "expected retry after reaching threshold, got {}",
        third_count
    );

    tracing::info!(log_index, "--- verify no snapshot was created");
    {
        let (_, mut sm) = router.get_storage_handle(&0)?;
        let snapshot = sm.get_current_snapshot().await?;
        assert!(
            snapshot.is_none(),
            "snapshot should not be created when building is disabled"
        );
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
