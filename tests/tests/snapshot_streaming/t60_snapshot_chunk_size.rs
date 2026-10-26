use anyhow::Result;
use openraft::Config;
use openraft::RaftLogReader;
use openraft::SnapshotPolicy;
use openraft::Vote;
use openraft::storage::RaftLogStorage;
use openraft::storage::RaftStateMachine;
use openraft_legacy::network_v1::InstallSnapshotRequest;
use openraft_legacy::network_v1::SnapshotMeta;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

const CHUNK_SIZE: usize = 10;

/// Test transfer snapshot in small chunks
///
/// What does this test do?
///
/// - build a stable single node cluster.
/// - send enough requests to the node that log compaction will be triggered.
/// - add learner and assert that they receive the snapshot and logs.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn snapshot_chunk_size() -> Result<()> {
    let snapshot_threshold: u64 = 10;

    let config = Config {
        snapshot_policy: SnapshotPolicy::LogsSinceLast(snapshot_threshold),
        snapshot_max_chunk_size: CHUNK_SIZE as u64,
        enable_heartbeat: false,
        max_in_snapshot_log_to_keep: 0,
        ..Default::default()
    };
    let builder = RaftRouter::builder(config)?;
    let builder = builder.chunked_snapshots();
    let mut router = builder.build();

    tracing::info!("--- bring up a single node cluster");
    let mut log_index = router.new_cluster([0], []).await?;

    tracing::info!(log_index, "--- send just enough logs to trigger snapshot");
    {
        router.client_request_many(0, "0", (snapshot_threshold - 1 - log_index) as usize).await?;
        log_index = snapshot_threshold - 1;

        router.wait(&0).applied_index(Some(log_index), "send log to trigger snapshot").await?;
        router.wait(&0).snapshot(log_id(1, 0, log_index), "snapshot").await?;

        let (mut sto, mut sm) = router.get_storage_handle(&0)?;
        assert_eq!(sto.get_log_state().await?.last_log_id, Some(log_id(1, 0, log_index)));
        assert_eq!(sto.read_vote().await?, Some(Vote::new_committed(1, 0)));

        let (last_applied, _) = sm.applied_state().await?;
        assert_eq!(last_applied, Some(log_id(1, 0, log_index)));

        let snap = sm.get_current_snapshot().await?.unwrap();
        assert_eq!(snap.meta.last_log_id, Some(log_id(1, 0, log_index)));

        let n0 = router.get_raft_handle(&0)?;
        n0.trigger().purge_log(log_index).await?;
        router.wait(&0).purged(Some(log_id(1, 0, 9)), "purge Leader-0 all in snapshot logs").await?;
    }

    tracing::info!(log_index, "--- add learner to receive snapshot and logs");
    {
        router.new_raft_node(1).await?;
        router.add_learner(0, 1).await.expect("failed to add new node as learner");
        log_index += 1;

        for id in [0, 1] {
            router.wait(&id).applied_index(Some(log_index), "add learner").await?;
        }
        router.wait(&1).applied_index(Some(log_index), "sync all data to learner-1").await?;
        router.wait(&1).snapshot(log_id(1, 0, log_index - 1), "learner-1 snapshot").await?;

        // after add_learner, log_index + 1,
        // leader has only log_index log in snapshot, cause it has compacted before add_learner
        {
            let (mut sto, mut sm) = router.get_storage_handle(&0)?;
            assert_eq!(sto.get_log_state().await?.last_log_id, Some(log_id(1, 0, log_index)));
            assert_eq!(sto.read_vote().await?, Some(Vote::new_committed(1, 0)));

            let (last_applied, _) = sm.applied_state().await?;
            assert_eq!(last_applied, Some(log_id(1, 0, log_index)));

            let snap = sm.get_current_snapshot().await?.unwrap();
            assert_eq!(snap.meta.last_log_id, Some(log_id(1, 0, log_index - 1)));
        }

        {
            let (mut sto, mut sm) = router.get_storage_handle(&1)?;
            assert_eq!(sto.get_log_state().await?.last_log_id, Some(log_id(1, 0, log_index)));
            assert_eq!(sto.read_vote().await?, Some(Vote::new_committed(1, 0)));

            let (last_applied, _) = sm.applied_state().await?;
            assert_eq!(last_applied, Some(log_id(1, 0, log_index)));

            let snap = sm.get_current_snapshot().await?.unwrap();
            assert_eq!(snap.meta.last_log_id, Some(log_id(1, 0, log_index - 1)));
        }
    }

    tracing::info!(log_index, "--- verify the exact chunks delivered by the legacy adapter");
    {
        let leader = router.get_raft_handle(&0)?;
        let snapshot = leader.get_snapshot().await?;
        let snapshot = snapshot.unwrap();
        let bytes = snapshot.snapshot.get_ref().clone();
        let meta = snapshot.meta.clone();
        let vote = Vote::new_committed(1, 0);
        let requests = router.snapshot_chunks();
        let snapshot_id = requests[0].meta.snapshot_id.clone();
        let wire_meta = SnapshotMeta::new(meta.clone(), snapshot_id);
        let mut expected = Vec::new();
        for (index, chunk) in bytes.chunks(CHUNK_SIZE).enumerate() {
            let offset = index * CHUNK_SIZE;
            let end = offset + chunk.len();
            let done = end == bytes.len();
            let request = InstallSnapshotRequest {
                vote,
                meta: wire_meta.clone(),
                offset: offset as u64,
                data: chunk.to_vec(),
                done,
            };
            expected.push(request);
        }
        assert_eq!(expected, requests);

        let node = router.get_raft_handle(&1)?;

        let installed = node.get_snapshot().await?;
        let installed = installed.unwrap();
        assert_eq!(meta, installed.meta);
        let installed_bytes = installed.snapshot.into_inner();
        assert_eq!(bytes, installed_bytes);
    }

    Ok(())
}
