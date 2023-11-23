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

/// Replication blocks purge, but it should not purge for ever.
/// Every new replication action should avoid using a log that is scheduled to be purged.
///
/// - Bring up one leader and two isolated learners. The leader keeps trying replicating logs to
///   learners.
/// - Trigger snapshot on the leader, logs should be able to be purged. Because replication should
///   avoid using a log id `i` that is `RaftState.last_purged_log_id() <= i <=
///   RaftState.purge_upto()`.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn replication_does_not_block_purge() -> Result<()> {
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
    let mut log_index = router.new_cluster(btreeset! {0}, btreeset! {1,2}).await?;

    let leader = router.get_raft_handle(&0)?;

    router.set_network_error(1, true);
    router.set_network_error(2, true);

    tracing::info!(log_index, "--- build snapshot on leader, check purged log");
    {
        log_index += router.client_request_many(0, "0", 10).await?;

        leader.trigger().snapshot().await?;
        leader
            .wait(timeout())
            .snapshot(LogId::new(CommittedLeaderId::new(1, 0), log_index), "built snapshot")
            .await?;

        sleep(Duration::from_millis(500)).await;

        let (mut sto0, mut _sm0) = router.get_storage_handle(&0)?;
        let logs = sto0.try_get_log_entries(..).await?;
        assert_eq!(max_keep as usize, logs.len(), "leader's local logs are purged");
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
