use std::time::Duration;

use anyhow::Result;
use openraft::Config;
use openraft::LogIdOptionExt;
use openraft::metrics::ReplicationWaitError;
use openraft::metrics::WaitError;

use crate::fixtures::RaftRouter;
use crate::fixtures::expect_timeout;
use crate::fixtures::timeout;
use crate::fixtures::ut_harness;

/// Test wait() utils
///
/// What does this test do?
///
/// - brings 1 nodes online:
/// - wait for expected state.
/// - wait for invalid state and expect a timeout error.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn metrics_wait() -> Result<()> {
    // Setup test dependencies.
    let config = Config {
        enable_tick: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- bring up a single node cluster");
    let log_index = router.new_cluster([0], []).await?;

    tracing::info!(log_index, "--- wait for a log that is never written, expect timeout");

    let never_written = log_index + 1;
    let msg = format!("timeout waiting for log {}", never_written);
    let duration = Duration::from_millis(20);
    let rst = router.wait_with_timeout(&0, duration).applied_index(Some(never_written), msg).await;
    expect_timeout(rst)?;

    tracing::info!("--- shutdown must fail a negative observation");
    {
        let node = router.get_raft_handle(&0)?;
        node.shutdown().await?;
        let result = router.wait(&0).applied_index(Some(never_written), "node stopped").await;
        let error = result.unwrap_err();
        assert_eq!(WaitError::ShuttingDown, error);
        let result = expect_timeout::<()>(Err(error));
        let error = result.unwrap_err();
        let wait_error = error.downcast_ref::<WaitError>();
        assert_eq!(Some(&WaitError::ShuttingDown), wait_error);
    }

    Ok(())
}

/// Replication waits can be reused after adding a learner, without another membership write.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn replication_wait() -> Result<()> {
    let config = Config {
        enable_tick: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- create a leader and learner and observe their initial replication");
    let (leader, membership_log_id) = {
        let log_index = router.new_cluster([0], [1]).await?;
        let leader = router.get_raft_handle(&0)?;
        let metrics = leader.wait(timeout()).replication(1, Some(log_index), 0).await?;
        (leader, *metrics.membership_config.log_id())
    };

    tracing::info!("--- block learner replication and write beyond its matched index");
    let log_index = {
        router.set_network_error(1, true);
        let response = router.client_request(0, "replication-wait", 0).await?;
        let log_index = response.log_id.index();
        let result = leader.wait(Some(Duration::ZERO)).replication(1, Some(log_index), 0).await;
        match result {
            Err(ReplicationWaitError::Wait(err)) => expect_timeout::<()>(Err(err))?,
            other => anyhow::bail!("expected replication timeout, got {other:?}"),
        }
        log_index
    };

    tracing::info!(
        log_index,
        "--- restore replication and wait without re-adding the learner"
    );
    {
        router.set_network_error(1, false);
        leader.trigger().heartbeat().await?;
        let metrics = leader.wait(timeout()).replication(1, Some(log_index), 0).await?;
        assert!(metrics.replication.as_ref().unwrap().get(&1).unwrap().index() >= Some(log_index));
        assert_eq!(&membership_log_id, metrics.membership_config.log_id());
    }

    tracing::info!(
        log_index,
        "--- reuse the wait after another write, with an explicit lag tolerance"
    );
    {
        let response = router.client_request(0, "replication-wait", 1).await?;
        let metrics = leader.wait(timeout()).replication(1, Some(response.log_id.index()), 10).await?;
        assert!(metrics.replication.as_ref().unwrap().get(&1).unwrap().index() >= Some(response.log_id.index()));
        assert_eq!(&membership_log_id, metrics.membership_config.log_id());
    }

    tracing::info!("--- reject waits on a learner and for a target outside membership");
    {
        let learner = router.get_raft_handle(&1)?;
        assert_eq!(
            ReplicationWaitError::NotLeader,
            learner.wait(timeout()).replication(0, None, 0).await.unwrap_err()
        );
        assert_eq!(
            ReplicationWaitError::NodeNotFound,
            leader.wait(timeout()).replication(99, None, 0).await.unwrap_err()
        );
    }

    Ok(())
}
