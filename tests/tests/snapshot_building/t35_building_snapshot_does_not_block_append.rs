use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::Vote;
use openraft::network::RPCOption;
use openraft::network::RaftNetworkFactory;
use openraft::network::v2::RaftNetworkV2;
use openraft::raft::AppendEntriesRequest;
use openraft::testing::blank_ent;
use openraft_memstore::BlockOperation;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// When building a snapshot, append-entries request should not be blocked.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn building_snapshot_does_not_block_append() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());
    let mut log_index = router.new_cluster(btreeset! {0,1}, btreeset! {}).await?;

    let follower = router.get_raft_handle(&1)?;

    tracing::info!(log_index, "--- set flag to block snapshot building");
    {
        let (mut _sto1, sm1) = router.get_storage_handle(&1)?;
        sm1.block.set_blocking(BlockOperation::BuildSnapshot, Duration::from_millis(5_000));
    }

    tracing::info!(log_index, "--- build snapshot on follower, it should block");
    {
        log_index += router.client_request_many(0, "0", 10).await?;
        router.wait(&1, timeout()).applied_index(Some(log_index), "written 10 logs").await?;

        follower.trigger().snapshot().await?;

        tracing::info!(log_index, "--- sleep 500 ms to make sure snapshot is started");
        tokio::time::sleep(Duration::from_millis(500)).await;

        let res = router
            .wait(&1, Some(Duration::from_millis(500)))
            .snapshot(log_id(1, 0, log_index), "building snapshot is blocked")
            .await;
        assert!(res.is_err(), "snapshot should be blocked and cannot finish");
    }

    tracing::info!(
        log_index,
        "--- send append-entries request to the follower that is building snapshot"
    );
    {
        let rpc = AppendEntriesRequest::<openraft_memstore::TypeConfig> {
            vote: Vote::new_committed(1, 0),
            prev_log_id: Some(log_id(1, 0, log_index)),
            entries: vec![blank_ent(1, 0, 15)],
            leader_commit: None,
        };

        let mut cli = router.new_client(1, &()).await;
        let option = RPCOption::new(Duration::from_millis(1_000));
        let fu = cli.append_entries(rpc, option);
        let fu = tokio::time::timeout(Duration::from_millis(500), fu);
        let resp = fu.await??;
        assert!(resp.is_success());
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
