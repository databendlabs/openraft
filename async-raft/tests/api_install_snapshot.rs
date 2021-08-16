mod fixtures;

use std::sync::Arc;

use anyhow::Result;
use async_raft::raft::InstallSnapshotRequest;
use async_raft::Config;
use async_raft::LogId;
use async_raft::SnapshotMeta;
use async_raft::State;
use fixtures::RaftRouter;
use maplit::btreeset;

///  API test: install_snapshot with various condition.
///
/// What does this test do?
///
/// - build a stable single node cluster.
/// - send install_snapshot request with matched/mismatched id and offset
///
/// export RUST_LOG=async_raft,memstore,snapshot_ge_half_threshold=trace
/// cargo test -p async-raft --test snapshot_ge_half_threshold
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn snapshot_ge_half_threshold() -> Result<()> {
    fixtures::init_tracing();

    let config = Arc::new(Config::build("test".into()).validate().expect("failed to build Raft config"));
    let router = Arc::new(RaftRouter::new(config.clone()));

    let mut want = 0;

    tracing::info!("--- initializing cluster");
    {
        router.new_raft_node(0).await;

        router.wait_for_log(&btreeset![0], want, None, "empty").await?;
        router.wait_for_state(&btreeset![0], State::NonVoter, None, "empty").await?;

        router.initialize_from_single_node(0).await?;
        want += 1;

        router.wait_for_log(&btreeset![0], want, None, "init leader").await?;
        router.assert_stable_cluster(Some(1), Some(want)).await;
    }

    let n = router.remove_node(0).await.ok_or_else(|| anyhow::anyhow!("node not found"))?;
    let req0 = InstallSnapshotRequest {
        term: 1,
        leader_id: 0,
        meta: SnapshotMeta {
            snapshot_id: "ss1".into(),
            last_log_id: LogId { term: 1, index: 0 },
            membership: Default::default(),
        },
        offset: 0,
        data: vec![1, 2, 3],
        done: false,
    };
    tracing::info!("--- install and write ss1:[0,3)");
    {
        let req = req0.clone();
        n.0.install_snapshot(req).await?;
    }

    tracing::info!("-- continue write with different id");
    {
        let mut req = req0.clone();
        req.offset = 3;
        req.meta.snapshot_id = "ss2".into();
        let res = n.0.install_snapshot(req).await;
        assert_eq!("expect: ss1+3, got: ss2+3", res.unwrap_err().to_string());
    }

    tracing::info!("-- write from offset=0 with different id, create a new session");
    {
        let mut req = req0.clone();
        req.offset = 0;
        req.meta.snapshot_id = "ss2".into();
        n.0.install_snapshot(req).await?;

        let mut req = req0.clone();
        req.offset = 3;
        req.meta.snapshot_id = "ss2".into();
        n.0.install_snapshot(req).await?;
    }

    tracing::info!("-- continue write with mismatched offset is allowed");
    {
        let mut req = req0.clone();
        req.offset = 8;
        req.meta.snapshot_id = "ss2".into();
        n.0.install_snapshot(req).await?;
    }
    Ok(())
}
