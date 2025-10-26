use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::ServerState;
use openraft::Vote;
use openraft::raft::FlushPoint;
use tokio::time::sleep;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// Test log progress API: get() and wait_until_ge()
///
/// What does this test do?
///
/// - Creates a single-node cluster
/// - Gets watch_log_progress() handle
/// - Verifies initial state with get()
/// - Writes client requests to generate logs
/// - Uses wait_until_ge() with concrete target value
/// - Verifies get() returns same value as wait_until_ge()
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn log_progress_api() -> Result<()> {
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

    tracing::info!(log_index, "--- get log progress watcher");
    let n0 = router.get_raft_handle(&0)?;
    let log_progress = n0.watch_log_progress();

    tracing::info!(log_index, "--- verify initial log progress with get()");
    let got = log_progress.get();
    let want = Some(FlushPoint::new(
        Vote::new_committed(1, 0),
        Some(log_id(1, 0, log_index)),
    ));
    assert_eq!(got, want);

    tracing::info!(log_index, "--- spawn task to wait for future log progress");
    let target_index = log_index + 5;
    let target = Some(FlushPoint::new(
        Vote::new_committed(1, 0),
        Some(log_id(1, 0, target_index)),
    ));

    let n0_clone = router.get_raft_handle(&0)?;
    let handle = tokio::spawn(async move {
        let mut progress = n0_clone.watch_log_progress();
        progress.wait_until_ge(&target).await
    });

    tracing::info!(log_index, "--- send client requests to trigger wait_until_ge return");
    log_index += router.client_request_many(0, "foo", 5).await?;

    tracing::info!(log_index, "--- verify wait_until_ge returns after writes are flushed");
    let got_wait = handle.await??;
    let got_get = log_progress.get();

    let want = Some(FlushPoint::new(
        Vote::new_committed(1, 0),
        Some(log_id(1, 0, log_index)),
    ));
    assert_eq!(got_wait, want);
    assert_eq!(got_get, want);

    Ok(())
}

/// Test log progress API with leader change
///
/// What does this test do?
///
/// - Initializes a 3-node cluster with node 0 as leader
/// - Gets watch_log_progress() handle on node 1
/// - Spawns task to wait for log progress with term 2 (new leader)
/// - Shuts down node 0 to force re-election
/// - Triggers election on node 1 to become new leader
/// - Writes client requests on new leader
/// - Verifies wait_until_ge() returns with new leader's vote and log
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn log_progress_with_leader_change() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing 3-node cluster");
    let mut log_index = router.new_cluster(btreeset! {0, 1, 2}, btreeset! {}).await?;

    tracing::info!(log_index, "--- get log progress watcher on node 1");
    let n1 = router.get_raft_handle(&1)?;
    let log_progress = n1.watch_log_progress();

    tracing::info!(log_index, "--- verify initial log progress with get()");
    let got = log_progress.get();
    let want = Some(FlushPoint::new(
        Vote::new_committed(1, 0),
        Some(log_id(1, 0, log_index)),
    ));
    assert_eq!(got, want);

    tracing::info!(log_index, "--- spawn task to wait for log progress with term 2");
    let target_index = log_index + 4;
    let target = Some(FlushPoint::new(
        Vote::new_committed(2, 1),
        Some(log_id(2, 1, target_index)),
    ));

    let n0 = router.get_raft_handle(&0)?;
    n0.shutdown().await?;

    // ensure node 0 is down and leader lease expire
    sleep(Duration::from_millis(500)).await;

    let n1_clone = router.get_raft_handle(&1)?;
    let handle = tokio::spawn(async move {
        let mut progress = n1_clone.watch_log_progress();
        progress.wait_until_ge(&target).await
    });

    tracing::info!(log_index, "--- shutdown node 0");
    router.remove_node(0);

    tracing::info!(log_index, "--- trigger election on node 1");
    let n1 = router.get_raft_handle(&1)?;
    n1.trigger().elect().await?;

    tracing::info!(log_index, "--- send client requests to new leader");
    router
        .wait(&1, Some(Duration::from_millis(2000)))
        .state(ServerState::Leader, "wait for node 1 to become leader")
        .await?;
    log_index += 1;
    log_index += router.client_request_many(1, "foo", 3).await?;

    tracing::info!(log_index, "--- verify wait_until_ge returns with new leader's progress");
    let got_wait = handle.await??;
    let got_get = log_progress.get();

    let want = Some(FlushPoint::new(
        Vote::new_committed(2, 1),
        Some(log_id(2, 1, log_index)),
    ));
    assert_eq!(got_wait, want);
    assert_eq!(got_get, want);

    Ok(())
}

/// Test vote progress API: get() and wait_until_ge() with leader change
///
/// What does this test do?
///
/// - Initializes a 3-node cluster with node 0 as leader
/// - Gets watch_vote() handle on node 1
/// - Spawns task to wait for future vote (term 2)
/// - Triggers election on node 1 to seize leadership
/// - Verifies wait_until_ge() returns when new leader is elected
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn vote_progress_api() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_tick: false,
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing 3-node cluster");
    let _log_index = router.new_cluster(btreeset! {0, 1, 2}, btreeset! {}).await?;

    tracing::info!("--- get vote progress watcher on node 1");
    let n1 = router.get_raft_handle(&1)?;
    let vote_progress = n1.watch_vote_progress();

    tracing::info!("--- verify initial vote progress with get()");
    let got = vote_progress.get();
    let want = Some(Vote::new_committed(1, 0));
    assert_eq!(got, want);

    tracing::info!("--- shutdown node 0");
    {
        let n0 = router.get_raft_handle(&0)?;
        n0.shutdown().await?;

        // ensure node 0 is down and leader lease expire
        sleep(Duration::from_millis(500)).await;
    }

    tracing::info!("--- spawn task to wait for term 2");
    let n1 = router.get_raft_handle(&1)?;
    let handle = tokio::spawn(async move {
        let mut progress = n1.watch_vote_progress();

        let target = Some(Vote::new(2, 1));
        progress.wait_until_ge(&target).await
    });

    tracing::info!("--- trigger election on node 1 to seize leadership");
    let n1 = router.get_raft_handle(&1)?;
    n1.trigger().elect().await?;

    tracing::info!("--- verify wait_until_ge returns with term 2 vote");
    let got_wait = handle.await??;
    let got_get = vote_progress.get();

    let want = Some(Vote::new(2, 1));
    assert_eq!(got_wait, want);
    assert_eq!(got_get, want);

    Ok(())
}
