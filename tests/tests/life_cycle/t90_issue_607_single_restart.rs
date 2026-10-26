use openraft::Config;

use crate::fixtures::Node;
use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// Brings up a cluster of 1 node and restart it.
///
/// Assert that `RaftCore.engine.state.server_state` and `RaftCore.leader_data` are consistent:
/// `server_state == Leader && leader_data.is_some() || server_state != Leader &&
/// leader_data.is_none()`.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn single_restart() -> anyhow::Result<()> {
    let config = Config {
        enable_heartbeat: false,
        ..Default::default()
    };

    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- bring up cluster of 1 node");
    let mut log_index = router.new_cluster([0], []).await?;

    tracing::info!(log_index, "--- write to 1 log");
    {
        router.client_request_many(0, "foo", 1).await?;
        log_index += 1;
    }

    tracing::info!(log_index, "--- stop and restart node 0");
    {
        let Node {
            log_store: sto,
            state_machine: sm,
            ..
        } = router.stop_node(0).await?;

        tracing::info!(log_index, "--- restart node 0");

        router.new_raft_node_with_sto(0, sto, sm).await?;
    }

    tracing::info!(log_index, "--- write to 1 log after restart");
    {
        router.client_request_many(0, "foo", 1).await?;
        log_index += 1;

        router.wait(&0).applied_index(Some(log_index), "node-0 works").await?;
    }

    Ok(())
}
