use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::Raft;
use openraft::ServerState;
use openraft::async_runtime::WatchReceiver;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// A built Raft node does not process state machine requests or campaign until `run()`.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn build_then_run() -> Result<()> {
    let config = Arc::new(
        Config {
            election_timeout_min: 100,
            election_timeout_max: 200,
            enable_heartbeat: false,
            enable_leader_restore: Some(false),
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- create an initialized node whose persisted membership permits campaigning");
    let (log_store, sm) = {
        router.new_cluster(btreeset! {0}, btreeset! {}).await?;
        let (node, log_store, sm) = router.remove_node(0).unwrap();
        node.shutdown().await?;
        (log_store, sm)
    };

    tracing::info!("--- build the node and queue a state machine request before run");
    let node = Raft::build(0, config, router, log_store, sm).await?;
    let invoked = Arc::new(AtomicBool::new(false));
    let invoked_for_worker = invoked.clone();
    {
        node.external_state_machine_request(move |_sm| {
            Box::pin(async move {
                invoked_for_worker.store(true, Ordering::SeqCst);
            })
        })
        .await?;

        TypeConfig::sleep(Duration::from_millis(400)).await;
        let invoked_before_run = invoked.load(Ordering::SeqCst);
        assert!(!invoked_before_run);
        let server_state = node.metrics().borrow_watched().state;
        assert_ne!(ServerState::Leader, server_state);
    }

    tracing::info!("--- run with ticks disabled; the queued state machine request executes");
    {
        node.runtime_config().tick(false);
        node.run();
        let worker_request = node.with_state_machine(|_sm| Box::pin(async {}));
        let worker_result = TypeConfig::timeout(Duration::from_secs(2), worker_request).await?;
        worker_result?;
        let invoked_after_run = invoked.load(Ordering::SeqCst);
        assert!(invoked_after_run);
        TypeConfig::sleep(Duration::from_millis(400)).await;
        let server_state = node.metrics().borrow_watched().state;
        assert_ne!(ServerState::Leader, server_state);
    }

    tracing::info!("--- enable ticks; the recovered single voter can now become leader");
    {
        node.runtime_config().tick(true);
        node.wait(timeout()).state(ServerState::Leader, "started by run").await?;
    }

    node.shutdown().await?;
    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_secs(2))
}
