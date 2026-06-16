use std::sync::Arc;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::errors::Fatal;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// Access Raft state via [`Raft::with_raft_state()`](openraft::Raft::with_raft_state)
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
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

    let committed = n0.with_raft_state(|st| st.local_committed().cloned()).await?;
    assert_eq!(committed, Some(log_id(1, 0, log_index)));

    // On the leader, the cluster-committed log id equals the local committed log id.
    let cluster_committed = n0.with_raft_state(|st| st.cluster_committed().cloned()).await?;
    assert_eq!(cluster_committed, Some(log_id(1, 0, log_index)));

    tracing::info!("--- shutting down node 0");
    n0.shutdown().await?;

    let res = n0.with_raft_state(|st| st.local_committed().cloned()).await;
    assert_eq!(Err(Fatal::Stopped), res);

    Ok(())
}
