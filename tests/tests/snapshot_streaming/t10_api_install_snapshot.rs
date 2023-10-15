use std::sync::Arc;

use anyhow::Result;
use maplit::btreeset;
use openraft::raft::ExampleChunkId;
use openraft::raft::ExampleManifest;
use openraft::raft::ExampleSnapshotChunk;
use openraft::raft::InstallSnapshotData;
use openraft::raft::InstallSnapshotRequest;
use openraft::CommittedLeaderId;
use openraft::Config;
use openraft::LogId;
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
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    let mut log_index = 0;

    tracing::info!(log_index, "--- initializing cluster");
    log_index = router.new_cluster(btreeset! {0}, btreeset! {}).await?;

    let n = router.remove_node(0).ok_or_else(|| anyhow::anyhow!("node not found"))?;
    let make_req = || InstallSnapshotRequest {
        // force it to be a follower
        vote: Vote::new_committed(2, 1),
        meta: SnapshotMeta {
            snapshot_id: "ss1".into(),
            last_log_id: Some(LogId {
                leader_id: CommittedLeaderId::new(1, 0),
                index: 0,
            }),
            last_membership: Default::default(),
        },
        data: InstallSnapshotData::Manifest(ExampleManifest::default()),
    };

    tracing::info!(log_index, "--- only allow to begin a new session when offset is 0");
    {
        let mut req = make_req();
        req.data = InstallSnapshotData::Chunk(ExampleSnapshotChunk {
            chunk_id: ExampleChunkId { offset: 2, len: 0 },
            data: vec![],
        });

        let res = n.0.install_snapshot(req).await;
        assert_eq!(
            "snapshot segment id mismatch, expect: , got: ss1",
            res.unwrap_err().to_string()
        );
    }

    tracing::info!(log_index, "--- install and write ss1:[0,3)");
    {
        n.0.install_snapshot(make_req()).await?;
    }

    tracing::info!("-- continue write with different id");
    {
        let mut req = make_req();
        req.meta.snapshot_id = "ss2".into();
        req.data = InstallSnapshotData::Chunk(ExampleSnapshotChunk {
            chunk_id: ExampleChunkId { offset: 3, len: 0 },
            data: vec![],
        });

        let res = n.0.install_snapshot(req).await;
        assert_eq!(
            "snapshot segment id mismatch, expect: ss1, got: ss2",
            res.unwrap_err().to_string()
        );
    }

    tracing::info!("-- write from offset=0 with different id, create a new session");
    {
        let mut req = make_req();
        req.meta.snapshot_id = "ss2".into();
        n.0.install_snapshot(req).await?;

        let mut req = make_req();
        req.data = InstallSnapshotData::Chunk(ExampleSnapshotChunk {
            chunk_id: ExampleChunkId { offset: 0, len: 0 },
            data: vec![],
        });
        req.meta.snapshot_id = "ss2".into();
        n.0.install_snapshot(req).await?;

        let mut req = make_req();
        req.data = InstallSnapshotData::Chunk(ExampleSnapshotChunk {
            chunk_id: ExampleChunkId { offset: 3, len: 0 },
            data: vec![],
        });
        req.meta.snapshot_id = "ss2".into();
        n.0.install_snapshot(req).await?;
    }

    tracing::info!("-- continue write with mismatched offset is allowed");
    {
        let mut req = make_req();
        req.data = InstallSnapshotData::Chunk(ExampleSnapshotChunk {
            chunk_id: ExampleChunkId { offset: 8, len: 0 },
            data: vec![],
        });

        req.meta.snapshot_id = "ss2".into();
        n.0.install_snapshot(req).await?;
    }
    Ok(())
}
