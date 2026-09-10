use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::ServerState;
use openraft::StepDownPolicy;
use openraft::storage::RaftLogStorage;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

const LEADER_LEASE: Duration = Duration::from_millis(200);
const GRACE_MILLIS: u64 = 100;
const WAIT_TIMEOUT: Duration = Duration::from_secs(2);

fn config(policy: StepDownPolicy) -> Result<Arc<Config>> {
    Ok(Arc::new(
        Config {
            election_timeout_min: 150,
            election_timeout_max: LEADER_LEASE.as_millis() as u64,
            heartbeat_interval: 50,
            enable_elect: false,
            quorum_loss_step_down: policy,
            ..Default::default()
        }
        .validate()?,
    ))
}

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn never_keeps_isolated_leader() -> Result<()> {
    let mut router = RaftRouter::new(config(StepDownPolicy::Never)?);

    tracing::info!("--- establish node 0 as Leader in a three-voter cluster");
    {
        router.new_cluster(btreeset! {0, 1, 2}, btreeset! {}).await?;
        router.wait(&0, Some(WAIT_TIMEOUT)).state(ServerState::Leader, "node 0 is Leader").await?;
    }

    tracing::info!("--- isolate node 0 beyond the leader-lease window");
    {
        router.set_network_error(0, true);
        TypeConfig::sleep(LEADER_LEASE + Duration::from_millis(GRACE_MILLIS * 2)).await;
    }

    tracing::info!("--- Never preserves the old Leader authority and writes no marker");
    {
        let metrics = router.get_metrics(&0)?;
        assert_eq!(ServerState::Leader, metrics.state);
        assert_eq!(Some(0), metrics.current_leader);

        let (mut log_store, _) = router.get_storage_handle(&0)?;
        assert_eq!(None, log_store.read_local_retirement().await?);
    }

    Ok(())
}

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn after_retires_isolated_leader() -> Result<()> {
    let mut router = RaftRouter::new(config(StepDownPolicy::After(GRACE_MILLIS))?);

    tracing::info!("--- establish node 0 as Leader and remember its authority");
    let retired_for = {
        router.new_cluster(btreeset! {0, 1, 2}, btreeset! {}).await?;
        let metrics = router.wait(&0, Some(WAIT_TIMEOUT)).state(ServerState::Leader, "node 0 is Leader").await?;
        *metrics.vote.leader_id()
    };

    tracing::info!("--- isolate node 0 so quorum evidence expires through the grace period");
    {
        router.set_network_error(0, true);
    }

    tracing::info!("--- the tick loop retires the old authority and persists its marker");
    {
        let metrics = router
            .wait(&0, Some(WAIT_TIMEOUT))
            .metrics(
                |m| m.state == ServerState::Follower && m.current_leader.is_none(),
                "node 0 retires without identifying itself as Leader",
            )
            .await?;
        assert_eq!(ServerState::Follower, metrics.state);
        assert_eq!(None, metrics.current_leader);

        let (mut log_store, _) = router.get_storage_handle(&0)?;
        assert_eq!(Some(retired_for), log_store.read_local_retirement().await?);
    }

    Ok(())
}
