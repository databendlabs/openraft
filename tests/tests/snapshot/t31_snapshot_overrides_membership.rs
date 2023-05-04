use std::option::Option::None;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::network::RPCOption;
use openraft::network::RaftNetwork;
use openraft::network::RaftNetworkFactory;
use openraft::raft::AppendEntriesRequest;
use openraft::storage::StorageHelper;
use openraft::testing::blank_ent;
use openraft::CommittedLeaderId;
use openraft::Config;
use openraft::EffectiveMembership;
use openraft::Entry;
use openraft::EntryPayload;
use openraft::LogId;
use openraft::Membership;
use openraft::SnapshotPolicy;
use openraft::Vote;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Test membership info is sync correctly along with snapshot.
///
/// What does this test do?
///
/// - build a stable single node cluster.
/// - send enough requests to the node that log compaction will be triggered.
/// - ensure that snapshot overrides the existent membership on the learner.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn snapshot_overrides_membership() -> Result<()> {
    let snapshot_threshold: u64 = 10;

    let config = Arc::new(
        Config {
            snapshot_policy: SnapshotPolicy::LogsSinceLast(snapshot_threshold),
            max_in_snapshot_log_to_keep: 0,
            purge_batch_size: 1,
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let mut log_index = router.new_cluster(btreeset! {0}, btreeset! {}).await?;

    tracing::info!(log_index, "--- send just enough logs to trigger snapshot");
    {
        router.client_request_many(0, "0", (snapshot_threshold - 1 - log_index) as usize).await?;
        log_index = snapshot_threshold - 1;

        router
            .wait_for_log(
                &btreeset![0],
                Some(log_index),
                timeout(),
                "send log to trigger snapshot",
            )
            .await?;

        router
            .wait_for_snapshot(
                &btreeset![0],
                LogId::new(CommittedLeaderId::new(1, 0), log_index),
                timeout(),
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
    }

    tracing::info!(log_index, "--- create learner");
    {
        tracing::info!(log_index, "--- create learner");
        router.new_raft_node(1).await;
        let (mut sto, mut sm) = router.get_storage_handle(&1)?;

        tracing::info!(log_index, "--- add a membership config log to the learner");
        {
            let req = AppendEntriesRequest {
                vote: Vote::new_committed(1, 0),
                prev_log_id: None,
                entries: vec![blank_ent(0, 0, 0), Entry {
                    log_id: LogId::new(CommittedLeaderId::new(1, 0), 1),
                    payload: EntryPayload::Membership(Membership::new(vec![btreeset! {2,3}], None)),
                }],
                leader_commit: Some(LogId::new(CommittedLeaderId::new(0, 0), 0)),
            };
            let option = RPCOption::new(Duration::from_millis(1_000));

            router.new_client(1, &()).await.append_entries(req, option).await?;

            tracing::info!(log_index, "--- check that learner membership is affected");
            {
                let m = StorageHelper::new(&mut sto, &mut sm).get_membership().await?;

                assert_eq!(&EffectiveMembership::default(), m.committed().as_ref());
                assert_eq!(
                    &Membership::new(vec![btreeset! {2,3}], None),
                    m.effective().membership()
                );
            }
        }

        tracing::info!(
            log_index,
            "--- add learner to the cluster to receive snapshot, which overrides the learner storage"
        );
        {
            let snapshot_index = log_index;

            router.add_learner(0, 1).await.expect("failed to add new node as learner");
            log_index += 1;

            tracing::info!(log_index, "--- DONE add learner");

            router.wait_for_log(&btreeset![0, 1], Some(log_index), timeout(), "add learner").await?;
            router
                .wait_for_snapshot(
                    &btreeset![1],
                    LogId::new(CommittedLeaderId::new(1, 0), snapshot_index),
                    timeout(),
                    "",
                )
                .await?;

            let expected_snap = Some((snapshot_index.into(), 1));

            router
                .assert_storage_state(
                    1,
                    log_index,
                    None, /* learner does not vote */
                    LogId::new(CommittedLeaderId::new(1, 0), log_index),
                    expected_snap,
                )
                .await?;

            let m = StorageHelper::new(&mut sto, &mut sm).get_membership().await?;

            assert_eq!(
                &Membership::new(vec![btreeset! {0}], Some(btreeset! {1})),
                m.committed().membership(),
                "membership should be overridden by the snapshot"
            );
            assert_eq!(
                &Membership::new(vec![btreeset! {0}], Some(btreeset! {1})),
                m.effective().membership(),
                "membership should be overridden by the snapshot"
            );
        }
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
