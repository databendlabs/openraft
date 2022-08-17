use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use fixtures::RaftRouter;
use maplit::btreeset;
use openraft::raft::InstallSnapshotRequest;
use openraft::Config;
use openraft::LogId;
use openraft::SnapshotMeta;
use openraft::State;

#[macro_use]
mod fixtures;

///  API test: install_snapshot with various condition.
///
/// What does this test do?
///
/// - build a stable single node cluster.
/// - send install_snapshot request with matched/mismatched id and offset
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn snapshot_ge_half_threshold() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    let config = Arc::new(Config::default().validate()?);
    let router = Arc::new(RaftRouter::new(config.clone()));

    let mut log_index = 0;

    tracing::info!("--- initializing cluster");
    {
        router.new_raft_node(0).await;

        router.wait_for_log(&btreeset![0], None, timeout(), "empty").await?;
        router.wait_for_state(&btreeset![0], State::Learner, timeout(), "empty").await?;

        router.initialize_from_single_node(0).await?;
        log_index += 1;

        router.wait_for_log(&btreeset![0], Some(log_index), timeout(), "init leader").await?;
        router.assert_stable_cluster(Some(1), Some(log_index)).await;
    }

    let n = router.remove_node(0).await.ok_or_else(|| anyhow::anyhow!("node not found"))?;
    let req0 = InstallSnapshotRequest {
        term: 1,
        leader_id: 0,
        meta: SnapshotMeta {
            snapshot_id: "ss1".into(),
            last_log_id: Some(LogId { term: 1, index: 0 }),
        },
        offset: 0,
        data: vec![1, 2, 3],
        done: false,
    };

    tracing::info!("--- only allow to begin a new session when offset is 0");
    {
        let mut req = req0.clone();
        req.offset = 2;
        let res = n.0.install_snapshot(req).await;
        assert_eq!(
            "snapshot segment id mismatch, expect: ss1+0, got: ss1+2",
            res.unwrap_err().to_string()
        );
    }

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
        assert_eq!(
            "snapshot segment id mismatch, expect: ss1+3, got: ss2+3",
            res.unwrap_err().to_string()
        );
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

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1000))
}
