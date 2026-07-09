use std::sync::Arc;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::ServerState;
use openraft::async_runtime::WatchReceiver;
use openraft::errors::Fatal;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// Shutdown raft node and check the metrics change.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn shutdown() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());
    let _log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    tracing::info!("--- performing node shutdowns");
    {
        for i in [0, 1, 2] {
            let (node, _, _) = router.remove_node(i).unwrap();
            node.shutdown().await?;
            let m = node.metrics();
            assert_eq!(ServerState::Shutdown, m.borrow_watched().state, "shutdown node-{}", i);
        }
    }

    Ok(())
}

/// A panicked RaftCore should also return a proper error the next time accessing the `Raft`.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn return_error_after_panic() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let log_index = router.new_cluster(btreeset! {0}, btreeset! {}).await?;
    let _ = log_index; // unused;

    tracing::info!(log_index, "--- panic the RaftCore");
    {
        router
            .external_request(0, |_s| {
                panic!("foo");
            })
            .await?;
    }

    tracing::info!(
        log_index,
        "--- calls the panicked raft should get a Fatal::Panicked error"
    );
    {
        let res = router.client_request(0, "foo", 2).await;
        let err = res.unwrap_err();
        assert_eq!(Fatal::Panicked, err.into_fatal().unwrap());
    }

    Ok(())
}

/// A state-machine worker that dies while RaftCore stays alive must not hang callers.
///
/// `get_snapshot()` sends a command straight to the state-machine worker's channel, bypassing
/// RaftCore. When the worker has panicked, resolving the stop cause must fall back to
/// `Fatal::Stopped` within the bounded wait, instead of joining the still-running core forever.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn get_snapshot_returns_stopped_when_sm_worker_dies() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing single-node cluster");
    let _log_index = router.new_cluster(btreeset! {0}, btreeset! {}).await?;

    tracing::info!("--- arm the state machine to panic on the next apply");
    {
        let (_log_store, sm) = router.get_storage_handle(&0)?;
        sm.panic_on_apply(true);
    }

    tracing::info!("--- a client write makes the SM worker panic and die; RaftCore stays up");
    {
        let res = router.client_request(0, "c", 1).await;
        assert!(
            res.is_err(),
            "write must fail: the SM worker panicked while applying it"
        );
    }

    tracing::info!("--- get_snapshot() must resolve to Fatal::Stopped, not hang");
    {
        let raft = router.get_raft_handle(&0)?;
        let err = raft.get_snapshot().await.unwrap_err();
        assert_eq!(Fatal::Stopped, err.into_fatal().unwrap());
    }

    Ok(())
}

/// After shutdown(), access to Raft should return a Fatal::Stopped error.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn return_error_after_shutdown() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let log_index = router.new_cluster(btreeset! {0}, btreeset! {}).await?;
    let _ = log_index; // unused;

    tracing::info!(log_index, "--- shutdown the raft");
    {
        let n = router.get_raft_handle(&0)?;
        n.shutdown().await?;
    }

    tracing::info!(
        log_index,
        "--- calls the panicked raft should get a Fatal::Panicked error"
    );
    {
        let res = router.client_request(0, "foo", 2).await;
        let err = res.unwrap_err();
        assert_eq!(Fatal::Stopped, err.into_fatal().unwrap());
    }

    Ok(())
}
