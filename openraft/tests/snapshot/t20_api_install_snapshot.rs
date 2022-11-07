use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::raft::InstallSnapshotRequest;
use openraft::Config;
use openraft::LeaderId;
use openraft::LogId;
use openraft::ServerState;
use openraft::SnapshotMeta;
use openraft::Vote;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

///  API test: install_snapshot with various arguments.
///
/// What does this test do?
///
/// - build a stable single node cluster.
/// - send install_snapshot request with matched/mismatched id and offset
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn snapshot_arguments() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    let mut log_index = 0;

    tracing::info!("--- initializing cluster");
    {
        router.new_raft_node(0);

        router.wait_for_log(&btreeset![0], None, timeout(), "empty").await?;
        router.wait_for_state(&btreeset![0], ServerState::Learner, timeout(), "empty").await?;

        router.initialize_from_single_node(0).await?;
        log_index += 1;

        router.wait_for_log(&btreeset![0], Some(log_index), timeout(), "init leader").await?;
        router.assert_stable_cluster(Some(1), Some(log_index));
    }

    let n = router.remove_node(0).ok_or_else(|| anyhow::anyhow!("node not found"))?;
    let req0 = InstallSnapshotRequest {
        vote: Vote::new_committed(1, 0),
        meta: SnapshotMeta {
            snapshot_id: "ss1".into(),
            last_log_id: Some(LogId {
                leader_id: LeaderId::new(1, 0),
                index: 0,
            }),
            last_membership: Default::default(),
        },
        data: vec![1, 2, 3],
    };

    tracing::info!("--- install and write ss1:[0,3)");
    {
        let req = req0.clone();
        n.0.install_snapshot(req).await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1000))
}
