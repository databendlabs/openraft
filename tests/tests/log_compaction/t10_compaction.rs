use std::option::Option::None;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::raft::AppendEntriesRequest;
use openraft::CommittedLeaderId;
use openraft::Config;
use openraft::Entry;
use openraft::EntryPayload;
use openraft::LogId;
use openraft::Membership;
use openraft::RaftNetwork;
use openraft::RaftNetworkFactory;
use openraft::RaftStorage;
use openraft::ServerState;
use openraft::SnapshotPolicy;
use openraft::StorageHelper;
use openraft::Vote;

use crate::fixtures::blank;
use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Compaction test.
///
/// What does this test do?
///
/// - build a stable single node cluster.
/// - send enough requests to the node that log compaction will be triggered.
/// - add new nodes and assert that they receive the snapshot.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn compaction() -> Result<()> {
    let snapshot_threshold: u64 = 50;

    // Setup test dependencies.
    let config = Arc::new(
        Config {
            snapshot_policy: SnapshotPolicy::LogsSinceLast(snapshot_threshold),
            max_in_snapshot_log_to_keep: 2,
            purge_batch_size: 1,
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());
    router.new_raft_node(0).await;

    let mut log_index = 0;

    // Assert all nodes are in learner state & have no entries.
    router.wait_for_log(&btreeset![0], None, timeout(), "empty").await?;
    router.wait_for_state(&btreeset![0], ServerState::Learner, timeout(), "empty").await?;

    router.assert_pristine_cluster();

    tracing::info!("--- initializing cluster");

    router.initialize_from_single_node(0).await?;
    log_index += 1;

    router.wait_for_log(&btreeset![0], Some(log_index), timeout(), "init leader").await?;
    router.assert_stable_cluster(Some(1), Some(1));

    // Send enough requests to the cluster that compaction on the node should be triggered.
    // Puts us exactly at the configured snapshot policy threshold.

    // Log at 0 count as 1
    router.client_request_many(0, "0", (snapshot_threshold - 1 - log_index) as usize).await?;
    log_index = snapshot_threshold - 1;

    router.wait_for_log(&btreeset![0], Some(log_index), timeout(), "write").await?;
    router.assert_stable_cluster(Some(1), Some(log_index));
    router
        .wait_for_snapshot(
            &btreeset![0],
            LogId::new(CommittedLeaderId::new(1, 0), log_index),
            None,
            "snapshot",
        )
        .await?;

    router
        .assert_storage_state(
            1,
            log_index,
            Some(0),
            LogId::new(CommittedLeaderId::new(1, 0), log_index),
            Some((log_index.into(), 1)),
        )
        .await?;

    // Add a new node and assert that it received the same snapshot.
    let mut sto1 = router.new_store();
    sto1.append_to_log(&[blank(0, 0), Entry {
        log_id: LogId::new(CommittedLeaderId::new(1, 0), 1),
        payload: EntryPayload::Membership(Membership::new(vec![btreeset! {0}], None)),
    }])
    .await?;

    router.new_raft_node_with_sto(1, sto1.clone()).await;
    router.add_learner(0, 1).await.expect("failed to add new node as learner");
    log_index += 1; // add_learner log

    tracing::info!("--- add 1 log after snapshot");
    {
        router.client_request_many(0, "0", 1).await?;
        log_index += 1;
    }

    router.wait_for_log(&btreeset![0, 1], Some(log_index), timeout(), "add follower").await?;

    tracing::info!("--- logs should be deleted after installing snapshot; left only the last one");
    {
        let mut sto = router.get_storage_handle(&1)?;
        let logs = StorageHelper::new(&mut sto).get_log_entries(..).await?;
        assert_eq!(2, logs.len());
        assert_eq!(LogId::new(CommittedLeaderId::new(1, 0), log_index - 1), logs[0].log_id)
    }

    // log 0 counts
    let expected_snap = Some(((snapshot_threshold - 1).into(), 1));
    router
        .assert_storage_state(
            1,
            log_index,
            None, /* learner does not vote */
            LogId::new(CommittedLeaderId::new(1, 0), log_index),
            expected_snap,
        )
        .await?;

    tracing::info!(
        "--- send a heartbeat with prev_log_id to be some value <= last_applied to ensure the commit index is updated"
    );
    {
        let res = router
            .new_client(1, &())
            .await
            .send_append_entries(AppendEntriesRequest {
                vote: Vote::new_committed(1, 0),
                prev_log_id: Some(LogId::new(CommittedLeaderId::new(1, 0), 2)),
                entries: vec![],
                leader_commit: Some(LogId::new(CommittedLeaderId::new(0, 0), 0)),
            })
            .await?;

        tracing::debug!("--- append-entries res: {:?}", res);

        assert!(res.is_success());
        assert!(!res.is_conflict());
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1000))
}
