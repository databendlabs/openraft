use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use openraft::Config;
use openraft::Raft;
use openraft::ServerState;
use openraft::TickDriver;
use openraft_memstore::TypeConfig;

use crate::fixtures::Node;
use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

fn recovered_config() -> Config {
    Config {
        election_timeout_min: 100,
        election_timeout_max: 200,
        enable_heartbeat: false,
        enable_leader_restore: Some(false),
        ..Default::default()
    }
}

/// Shutting the shared driver down before `run` leaves no internal ticker.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn shared_driver_shutdown_before_run_does_not_elect() -> Result<()> {
    let config = recovered_config();
    let mut router = RaftRouter::new(config.clone())?;

    tracing::info!("--- recover a single voter that campaigns only when a tick arrives");
    let (log_store, sm) = {
        router.new_cluster([0], []).await?;
        let Node {
            log_store,
            state_machine: sm,
            ..
        } = router.stop_node(0).await?;
        (log_store, sm)
    };

    tracing::info!("--- run against a driver that has already stopped");
    {
        let driver = TickDriver::<TypeConfig>::spawn();
        let node = Raft::build(0, Arc::new(config), router, log_store, sm).await?;
        node.use_tick_driver(&driver)?;
        driver.shutdown().await?;
        node.runtime_config().tick(true);
        node.run();
        let waited = node.wait(Some(Duration::from_secs(2))).state(ServerState::Leader, "no internal ticker").await;
        let still_follower = waited.is_err();
        assert!(still_follower);
        node.shutdown().await?;
    }

    Ok(())
}

/// A live shared driver wakes a recovered sole voter without an internal ticker.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn shared_driver_elects_recovered_sole_voter() -> Result<()> {
    let config = recovered_config();
    let mut router = RaftRouter::new(config.clone())?;

    tracing::info!("--- recover a single voter");
    let (log_store, sm) = {
        router.new_cluster([0], []).await?;
        let Node {
            log_store,
            state_machine: sm,
            ..
        } = router.stop_node(0).await?;
        (log_store, sm)
    };

    tracing::info!("--- the shared driver delivers the immediate sole-voter wake");
    {
        let driver = TickDriver::<TypeConfig>::spawn();
        let node = Raft::build(0, Arc::new(config), router, log_store, sm).await?;
        let attached = node.use_tick_driver(&driver);
        assert_eq!(Ok(()), attached);
        node.run();
        node.wait(Some(Duration::from_secs(2))).state(ServerState::Leader, "shared driver tick").await?;
        node.shutdown().await?;
        driver.shutdown().await?;
    }

    Ok(())
}

/// `runtime_config().tick` pauses and resumes the shared driver.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn shared_driver_tick_flag_pauses_and_resumes() -> Result<()> {
    let config = recovered_config();
    let mut router = RaftRouter::new(config.clone())?;

    tracing::info!("--- recover a single voter");
    let (log_store, sm) = {
        router.new_cluster([0], []).await?;
        let Node {
            log_store,
            state_machine: sm,
            ..
        } = router.stop_node(0).await?;
        (log_store, sm)
    };

    tracing::info!("--- a paused driver does not elect; resuming it does");
    {
        let driver = TickDriver::<TypeConfig>::spawn();
        let node = Raft::build(0, Arc::new(config), router, log_store, sm).await?;
        node.use_tick_driver(&driver)?;
        node.runtime_config().tick(false);
        node.run();
        let paused =
            node.wait(Some(Duration::from_millis(500))).state(ServerState::Leader, "paused shared driver").await;
        let still_follower = paused.is_err();
        assert!(still_follower);

        node.runtime_config().tick(true);
        node.wait(Some(Duration::from_secs(2))).state(ServerState::Leader, "resumed shared driver").await?;
        node.shutdown().await?;
        driver.shutdown().await?;
    }

    Ok(())
}
