use std::time::Duration;

use anyhow::Result;
use openraft::Config;
use openraft::Vote;
use openraft::async_runtime::WatchReceiver;
use openraft::raft::AppendEntriesRequest;
use openraft::testing::blank_ent;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::BlockOperation;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::timeout;
use crate::fixtures::ut_harness;

/// When building a snapshot, applying-entries request should not be blocked.
///
/// Issue: https://github.com/databendlabs/openraft/issues/596
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn building_snapshot_does_not_block_apply() -> Result<()> {
    let config = Config {
        enable_tick: false,
        ..Default::default()
    };

    let mut router = RaftRouter::new(config)?;
    let mut log_index = router.new_cluster([0, 1], []).await?;

    let follower = router.get_raft_handle(&1)?;

    tracing::info!(log_index, "--- set flag to delay snapshot building");
    let gate = {
        let (_sto1, sm1) = router.get_storage_handle(&1)?;
        sm1.block.pause(BlockOperation::DelayBuildingSnapshot)
    };

    tracing::info!(log_index, "--- build snapshot on follower, it should block");
    {
        let written = router.client_request_many(0, "0", 10).await?;
        log_index += written.count;
        router.wait(&1).applied_index(Some(log_index), "written 10 logs").await?;

        follower.trigger().snapshot().await?;

        TypeConfig::timeout(timeout().unwrap(), gate.entered()).await?;
        let metrics = follower.metrics();
        let snapshot = metrics.borrow_watched().snapshot;
        assert_eq!(None, snapshot);
    }

    tracing::info!(
        log_index,
        "--- send append-entries request to the follower that is building snapshot"
    );
    {
        let next = log_index + 1;

        let rpc = AppendEntriesRequest::<openraft_memstore::TypeConfig> {
            vote: Vote::new_committed(1, 0),
            prev_log_id: Some(log_id(1, 0, log_index)),
            entries: vec![blank_ent::<openraft_memstore::TypeConfig>(1, 0, next)],
            // Append and commit this entry
            leader_commit: Some(log_id(1, 0, next)),
        };

        let node = router.get_raft_handle(&1)?;
        let fu = node.append_entries(rpc);
        let fu = TypeConfig::timeout(Duration::from_millis(500), fu);
        let resp = fu.await??;
        assert!(resp.is_success());

        router
            .wait(&1)
            .applied_index(
                Some(next),
                format!("log at index {} can be applied, while snapshot is building", next),
            )
            .await?;
    }

    tracing::info!(
        log_index,
        "--- release snapshot building after the independent operation completes"
    );
    {
        gate.release();
        let snapshot_log_id = log_id(1, 0, log_index + 1);
        router.wait(&1).snapshot(snapshot_log_id, "released snapshot completes").await?;
    }

    Ok(())
}
