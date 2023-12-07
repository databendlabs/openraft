use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::CommittedLeaderId;
use openraft::Config;
use openraft::LogId;
use openraft::RaftLogReader;
use tokio::time::sleep;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Leader logs should be deleted upto snapshot.last_log_id-max_in_snapshot_log_to_keep after
/// building snapshot; Follower/learner should delete upto snapshot.last_log_id after installing
/// snapshot.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn purge_in_snapshot_logs() -> Result<()> {
    let max_keep = 2;

    let config = Arc::new(
        Config {
            max_in_snapshot_log_to_keep: max_keep,
            purge_batch_size: 1,
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());
    let mut log_index = router.new_cluster(btreeset! {0}, btreeset! {1}).await?;

    let leader = router.get_raft_handle(&0)?;
    let learner = router.get_raft_handle(&1)?;

    let (mut sto0, mut _sm0) = router.get_storage_handle(&0)?;

    tracing::info!(log_index, "--- build snapshot on leader, check purged log");
    {
        log_index += router.client_request_many(0, "0", 10).await?;
        leader.trigger().snapshot().await?;
        leader
            .wait(timeout())
            .snapshot(
                LogId::new(CommittedLeaderId::new(1, 0), log_index),
                "building 1st snapshot",
            )
            .await?;
        let (mut sto0, mut _sm0) = router.get_storage_handle(&0)?;

        // Wait for purge to complete.
        sleep(Duration::from_millis(500)).await;

        let logs = sto0.try_get_log_entries(..).await?;
        assert_eq!(max_keep as usize, logs.len());
    }

    // Leader:  -------15..20
    // Learner: 0..10
    tracing::info!(log_index, "--- block replication, build another snapshot");
    {
        router.set_network_error(1, true);

        log_index += router.client_request_many(0, "0", 5).await?;
        router.wait(&0, timeout()).applied_index(Some(log_index), "write another 5 logs").await?;

        leader.trigger().snapshot().await?;
        leader
            .wait(timeout())
            .snapshot(
                LogId::new(CommittedLeaderId::new(1, 0), log_index),
                "building 2nd snapshot",
            )
            .await?;
    }

    // There may be a cached append-entries request that already loads log 10..15 from the store,
    // just before building snapshot.
    sleep(Duration::from_millis(500)).await;

    tracing::info!(
        log_index,
        "--- restore replication, install the 2nd snapshot on learner"
    );
    {
        router.set_network_error(1, false);

        learner
            .wait(timeout())
            .snapshot(
                LogId::new(CommittedLeaderId::new(1, 0), log_index),
                "learner install snapshot",
            )
            .await?;

        let (mut sto1, mut _sm) = router.get_storage_handle(&1)?;
        let logs = sto1.try_get_log_entries(..).await?;
        assert_eq!(0, logs.len());
    }

    // finally logs are purged on leader.
    let logs = sto0.try_get_log_entries(..).await?;
    assert_eq!(
        log_index + 1 - max_keep,
        logs[0].log_id.index,
        "leader's local logs are purged"
    );

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
