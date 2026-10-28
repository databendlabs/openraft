use std::time::Duration;

use anyhow::Result;
use openraft::Config;
use openraft::RaftLogReader;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::timeout;
use crate::fixtures::ut_harness;

/// Leader logs should be deleted up to snapshot.last_log_id-max_in_snapshot_log_to_keep after
/// building snapshot; Follower/learner should delete up to snapshot.last_log_id after installing
/// snapshot.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn purge_in_snapshot_logs() -> Result<()> {
    let max_keep = 2;

    let config = Config {
        max_in_snapshot_log_to_keep: max_keep,
        purge_batch_size: 1,
        enable_tick: false,
        ..Default::default()
    };

    let mut router = RaftRouter::new(config)?;
    let mut log_index = router.new_cluster([0], [1]).await?;

    let leader = router.get_raft_handle(&0)?;
    let learner = router.get_raft_handle(&1)?;

    let (mut sto0, mut _sm0) = router.get_storage_handle(&0)?;

    tracing::info!(log_index, "--- build snapshot on leader, check purged log");
    {
        let written = router.client_request_many(0, "0", 10).await?;
        log_index += written.count;
        leader.trigger().snapshot().await?;
        leader.wait(timeout()).snapshot(log_id(1, 0, log_index), "building 1st snapshot").await?;

        let purged_index = log_index - max_keep;
        let purged = log_id(1, 0, purged_index);
        leader.wait(timeout()).purged(Some(purged), "purged after 1st snapshot").await?;

        let (mut sto0, mut _sm0) = router.get_storage_handle(&0)?;
        let logs = sto0.try_get_log_entries(..).await?;
        let mut got = Vec::new();
        for entry in &logs {
            got.push(entry.log_id.clone());
        }

        let first = log_index + 1 - max_keep;
        let mut want = Vec::new();
        let mut index = first;
        while index <= log_index {
            want.push(log_id(1, 0, index));
            index += 1;
        }
        assert_eq!(want, got, "leader keeps max_in_snapshot_log_to_keep logs");
    }

    // Leader:  -------15..20
    // Learner: 0..10
    tracing::info!(log_index, "--- block replication, build another snapshot");
    {
        router.set_network_error(1, true);

        let written = router.client_request_many(0, "0", 5).await?;
        log_index += written.count;
        router.wait(&0).applied_index(Some(log_index), "write another 5 logs").await?;

        leader.trigger().snapshot().await?;
        leader.wait(timeout()).snapshot(log_id(1, 0, log_index), "building 2nd snapshot").await?;
    }

    // There may be a cached append-entries request that already loads log 10..15 from the store,
    // just before building snapshot.
    TypeConfig::sleep(Duration::from_millis(2_000)).await;

    tracing::info!(
        log_index,
        "--- restore replication, install the 2nd snapshot on learner"
    );
    {
        router.set_network_error(1, false);

        learner.wait(timeout()).snapshot(log_id(1, 0, log_index), "learner install snapshot").await?;

        let (mut sto1, mut _sm) = router.get_storage_handle(&1)?;
        let logs = sto1.try_get_log_entries(..).await?;
        assert_eq!(0, logs.len());
    }

    // finally logs are purged on leader.
    let purged_index = log_index - max_keep;
    let purged = log_id(1, 0, purged_index);
    leader.wait(timeout()).purged(Some(purged), "purged after 2nd snapshot").await?;

    let logs = sto0.try_get_log_entries(..).await?;
    let mut got = Vec::new();
    for entry in &logs {
        got.push(entry.log_id.clone());
    }

    let first = log_index + 1 - max_keep;
    let mut want = Vec::new();
    let mut index = first;
    while index <= log_index {
        want.push(log_id(1, 0, index));
        index += 1;
    }
    assert_eq!(want, got, "leader's local logs are purged");

    Ok(())
}
