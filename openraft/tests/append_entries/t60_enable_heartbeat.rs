use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Enable heartbeat, heartbeat log should be replicated.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn enable_heartbeat() -> Result<()> {
    // Setup test dependencies.
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let mut log_index = router.new_nodes_from_single(btreeset! {0,1,2}, btreeset! {3}).await?;

    let node0 = router.get_raft_handle(&0)?;
    node0.enable_heartbeat(true);

    for _i in 0..10 {
        log_index += 1; // new heartbeat log
        router.wait(&0, timeout()).log_at_least(Some(log_index), "node 0 emit heartbeat log").await?;
        router.wait(&1, timeout()).log_at_least(Some(log_index), "node 1 receives heartbeat").await?;
        router.wait(&3, timeout()).log_at_least(Some(log_index), "node 1 receives heartbeat").await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
