use std::sync::Arc;

use anyhow::Result;
use maplit::btreeset;
use openraft::error::Fatal;
use openraft::Config;
use openraft::ServerState;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Shutdown raft node and check the metrics change.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
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
            assert_eq!(ServerState::Shutdown, m.borrow().state, "shutdown node-{}", i);
        }
    }

    Ok(())
}

/// A panicked RaftCore should also return a proper error the next time accessing the `Raft`.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
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
        router.external_request(0, |_s| {
            panic!("foo");
        });
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

/// After shutdown(), access to Raft should return a Fatal::Stopped error.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
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
