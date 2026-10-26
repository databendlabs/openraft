use anyhow::Result;
use openraft::Config;
use openraft::ServerState;
use openraft::errors::Fatal;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// Access Raft state via [`Raft::with_raft_state()`](openraft::Raft::with_raft_state)
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn with_raft_state() -> Result<()> {
    let config = Config {
        enable_heartbeat: false,
        ..Default::default()
    };

    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initializing cluster");
    let log_index = {
        router.new_raft_node(0).await?;
        let node = router.get_raft_handle(&0)?;
        node.external_request(|state| assert_eq!(ServerState::Learner, state.server_state)).await?;
        router.initialize(0).await?;
        router.wait(&0).applied_index(Some(1), "initial leader blank").await?;
        for id in [1, 2] {
            router.new_raft_node(id).await?;
            router.add_learner(0, id).await?;
        }
        let response = node.change_membership([0, 1, 2], false).await?;
        let index = response.log_id.index();
        router.wait_applied_at_least([0, 1, 2], index, "cluster ready").await?;
        index
    };

    let n0 = router.get_raft_handle(&0)?;

    tracing::info!(log_index, "--- inspect the leader's committed log IDs");
    {
        let committed = n0.with_raft_state(|st| st.local_committed().cloned()).await?;
        assert_eq!(Some(log_id(1, 0, log_index)), committed);

        let cluster_committed = n0.with_raft_state(|st| st.cluster_committed().cloned()).await?;
        assert_eq!(Some(log_id(1, 0, log_index)), cluster_committed);
    }

    tracing::info!("--- shutting down node 0");
    {
        n0.shutdown().await?;

        let res = n0.with_raft_state(|st| st.local_committed().cloned()).await;
        assert_eq!(Err(Fatal::Stopped), res);
    }

    Ok(())
}
