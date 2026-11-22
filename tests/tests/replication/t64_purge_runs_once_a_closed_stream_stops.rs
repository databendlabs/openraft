use anyhow::Result;
use maplit::btreeset;
use openraft::ChangeMembers;
use openraft::Config;
use openraft::SnapshotPolicy;
use openraft::async_runtime::WatchReceiver;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// A purge requested right after a membership change closes a replication stream runs once that
/// stream's task has stopped.
///
/// The purge waits for the closed task, which may still be reading the log, and the task's join
/// reports when it has stopped. This case checks that the report arrives and the postponed purge
/// runs: node 2 is cut off, so its task keeps retrying from index 4 until node 2 is removed.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn purge_runs_once_a_closed_stream_stops() -> Result<()> {
    let config = Config {
        enable_heartbeat: false,
        enable_elect: false,
        snapshot_policy: SnapshotPolicy::Never,
        max_in_snapshot_log_to_keep: u64::MAX,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;
    let mut log_index = router.new_cluster([0], [1, 2]).await?;
    let n0 = router.get_raft_handle(&0)?;

    tracing::info!(
        log_index,
        "--- cut off node 2 and write 5 logs, which only node 1 receives"
    );
    {
        router.set_network_error(2, true);
        let written = router.client_request_many(0, "foo", 5).await?;
        log_index += written.count;
        router.wait(&1).applied_index(Some(log_index), "node 1 applies the writes").await?;
    }

    tracing::info!(
        log_index,
        "--- remove node 2, which closes its stream, and purge every log at once"
    );
    {
        n0.change_membership(ChangeMembers::RemoveNodes(btreeset! {2}), false).await?;
        log_index += 1;

        n0.trigger().snapshot().await?;
        router.wait(&0).snapshot(log_id(1, 0, log_index), "node 0 builds a snapshot").await?;
        n0.trigger().purge_log(log_index).await?;
    }

    tracing::info!(log_index, "--- the closed task stops, and the purge runs");
    {
        router.wait(&0).purged(Some(log_id(1, 0, log_index)), "the purge runs").await?;

        let running_state = n0.metrics().borrow_watched().running_state.clone();
        assert_eq!(Ok(()), running_state, "node 0 keeps running");
    }

    Ok(())
}
