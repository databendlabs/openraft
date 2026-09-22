use std::sync::Arc;
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

/// A built Raft node does not campaign until `run()` starts its core task.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn build_then_run() -> Result<()> {
    let config = Arc::new(
        Config {
            election_timeout_min: 100,
            election_timeout_max: 200,
            enable_heartbeat: false,
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

    tracing::info!("--- build the node and leave its core unstarted for longer than an election timeout");
    let node = Raft::build(0, config, router, log_store, sm).await?;
    {
        TypeConfig::sleep(Duration::from_millis(400)).await;
        assert_ne!(ServerState::Leader, node.metrics().borrow_watched().state);
    }

    tracing::info!("--- run the core; the recovered single voter can now become leader");
    {
        node.run();
        node.wait(timeout()).state(ServerState::Leader, "started by run").await?;
    }

    node.shutdown().await?;
    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_secs(2))
}
