use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::Entry;
use openraft::EntryPayload;
use openraft::Membership;
use openraft::RaftLogReader;
use openraft::SnapshotPolicy;
use openraft::Vote;
use openraft::network::RPCOption;
use openraft::network::RaftNetworkFactory;
use openraft::network::v2::RaftNetworkV2;
use openraft::raft::AppendEntriesRequest;
use openraft::storage::RaftLogStorageExt;
use openraft::testing::blank_ent;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// Compaction test.
///
/// What does this test do?
///
/// - build a stable single node cluster.
/// - send enough requests to the node that log compaction will be triggered.
/// - add new nodes and assert that they receive the snapshot.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn build_snapshot() -> Result<()> {
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

    tracing::info!("--- initializing cluster");
    let mut log_index = router.new_cluster(btreeset! {0}, btreeset! {}).await?;

    // Send enough requests to the cluster that compaction on the node should be triggered.
    // Puts us exactly at the configured snapshot policy threshold.

    // Log at 0 count as 1
    router.client_request_many(0, "0", (snapshot_threshold - 1 - log_index) as usize).await?;
    log_index = snapshot_threshold - 1;

    tracing::info!(log_index, "--- log_index: {}", log_index);

    router.wait(&0, timeout()).applied_index(Some(log_index), "write").await?;
    router.wait(&0, None).snapshot(log_id(1, 0, log_index), "snapshot").await?;

    router
        .assert_storage_state(
            1,
            log_index,
            Some(0),
            log_id(1, 0, log_index),
            Some((log_index.into(), 1)),
        )
        .await?;

    // Add a new node and assert that it received the same snapshot.
    let (mut sto1, sm1) = router.new_store();
    sto1.blocking_append([blank_ent(0, 0, 0), Entry {
        log_id: log_id(1, 0, 1),
        payload: EntryPayload::Membership(Membership::new_with_defaults(vec![btreeset! {0}], [])),
    }])
    .await?;

    router.new_raft_node_with_sto(1, sto1.clone(), sm1.clone()).await;
    router.add_learner(0, 1).await.expect("failed to add new node as learner");
    log_index += 1; // add_learner log

    tracing::info!(log_index, "--- add 1 log after snapshot, log_index: {}", log_index);
    {
        router.client_request_many(0, "0", 1).await?;
        log_index += 1;
    }

    tracing::info!(log_index, "--- log_index: {}", log_index);

    for id in [0, 1] {
        router.wait(&id, timeout()).applied_index(Some(log_index), "add follower").await?;
    }

    tracing::info!(
        log_index,
        "--- logs should be deleted after installing snapshot; left only the last one"
    );
    {
        let (mut sto, _sm) = router.get_storage_handle(&1)?;
        let logs = sto.try_get_log_entries(..).await?;
        assert_eq!(2, logs.len());
        assert_eq!(log_id(1, 0, log_index - 1), logs[0].log_id)
    }

    // log 0 counts
    let expected_snap = Some(((snapshot_threshold - 1).into(), 1));
    router
        .assert_storage_state(
            1,
            log_index,
            None, /* learner does not vote */
            log_id(1, 0, log_index),
            expected_snap,
        )
        .await?;

    tracing::info!(
        "--- send a heartbeat with prev_log_id to be some value <= last_applied to ensure the commit index is updated"
    );
    {
        let option = RPCOption::new(Duration::from_millis(1_000));

        let res = router
            .new_client(1, &())
            .await
            .append_entries(
                AppendEntriesRequest {
                    vote: Vote::new_committed(1, 0),
                    prev_log_id: Some(log_id(1, 0, 2)),
                    entries: vec![],
                    leader_commit: Some(log_id(0, 0, 0)),
                },
                option,
            )
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
