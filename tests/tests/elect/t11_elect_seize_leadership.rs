use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::ServerState;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// A node with higher term takes leadership from the current leader.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn elect_seize_leadership() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- create cluster of 0,1,2");
    let log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    let n0 = router.get_raft_handle(&0)?;
    n0.wait(timeout()).state(ServerState::Leader, "node 0 becomes leader").await?;

    tracing::info!(log_index, "--- trigger election on node 1");
    {
        let n1 = router.get_raft_handle(&1)?;
        n1.trigger().elect().await?;

        n1.wait(timeout()).state(ServerState::Leader, "node 1 becomes leader").await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2000))
}
