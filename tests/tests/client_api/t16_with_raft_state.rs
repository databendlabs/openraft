use std::sync::Arc;

use anyhow::Result;
use maplit::btreeset;
use openraft::error::Fatal;
use openraft::testing::log_id;
use openraft::Config;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Access Raft state via `Raft::with_raft_state()`
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn with_raft_state() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    let n0 = router.get_raft_handle(&0)?;

    let committed = n0.with_raft_state(|st| st.committed).await?;
    assert_eq!(committed, Some(log_id(1, 0, log_index)));

    tracing::info!("--- shutting down node 0");
    n0.shutdown().await?;

    let res = n0.with_raft_state(|st| st.committed).await;
    assert_eq!(Err(Fatal::Stopped), res);

    Ok(())
}
