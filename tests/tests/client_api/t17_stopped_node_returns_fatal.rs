use anyhow::Result;
use openraft::Config;
use openraft::ReadPolicy;
use openraft::errors::Fatal;
use openraft::errors::RaftError;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// Router helpers return `RaftError::Fatal(Fatal::Stopped)` for a stopped, registered node.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn stopped_node_returns_fatal() -> Result<()> {
    let config = Config {
        enable_local_lease_read: Some(true),
        enable_heartbeat: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- bring up a single-node cluster");
    let log_index = router.new_cluster([0], []).await?;

    tracing::info!("--- shut down node 0 and keep it in the router");
    {
        let node = router.get_raft_handle(&0)?;
        let cached = node.ensure_linearizable(ReadPolicy::LeaseRead).await?;
        let cached_index = cached.index();
        assert_eq!(log_index, cached_index);
        node.shutdown().await?;
        router.get_raft_handle(&0)?;
    }

    tracing::info!("--- add_learner on the stopped node returns Fatal::Stopped");
    {
        let add_learner = router.add_learner(0, 1).await;
        let add_status = add_learner.map(|_| ());
        assert_eq!(Err(RaftError::Fatal(Fatal::Stopped)), add_status);
    }

    tracing::info!("--- get_read_log_id on the stopped node returns Fatal::Stopped");
    for policy in [ReadPolicy::ReadIndex, ReadPolicy::LeaseRead] {
        let read_log_id = router.get_read_log_id(0, policy).await;
        // `LinearizableReadError` has no `PartialEq`, so keep only the fatal arm comparable.
        let read_status = match read_log_id {
            Ok(_) => Ok(()),
            Err(RaftError::Fatal(fatal)) => Err(RaftError::Fatal(fatal)),
            Err(RaftError::APIError(api_error)) => Err(RaftError::APIError(format!("{api_error:?}"))),
        };
        assert_eq!(Err(RaftError::Fatal(Fatal::Stopped)), read_status);
    }

    Ok(())
}
