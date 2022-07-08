use std::option::Option::None;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::raft::AppendEntriesRequest;
use openraft::storage::StorageHelper;
use openraft::Config;
use openraft::EffectiveMembership;
use openraft::Entry;
use openraft::EntryPayload;
use openraft::LeaderId;
use openraft::LogId;
use openraft::Membership;
use openraft::RaftNetwork;
use openraft::RaftNetworkFactory;
use openraft::SnapshotPolicy;
use openraft::Vote;

use crate::fixtures::blank;
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
            max_applied_log_to_keep: 0,
            purge_batch_size: 1,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let mut log_index = router.new_nodes_from_single(btreeset! {0}, btreeset! {}).await?;

    tracing::info!("--- send just enough logs to trigger snapshot");
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
        router.assert_stable_cluster(Some(1), Some(log_index));

        router
            .wait_for_snapshot(
                &btreeset![0],
                LogId::new(LeaderId::new(1, 0), log_index),
                timeout(),
                "snapshot",
            )
            .await?;
        router
            .assert_storage_state(
                1,
                log_index,
                Some(0),
                LogId::new(LeaderId::new(1, 0), log_index),
                Some((log_index.into(), 1)),
            )
            .await?;
    }

    tracing::info!("--- create learner");
    {
        tracing::info!("--- create learner");
        router.new_raft_node(1);
        let mut sto = router.get_storage_handle(&1)?;

        tracing::info!("--- add a membership config log to the learner");
        {
            let req = AppendEntriesRequest {
                vote: Vote::new_committed(1, 0),
                prev_log_id: None,
                entries: vec![blank(0, 0), Entry {
                    log_id: LogId::new(LeaderId::new(1, 0), 1),
                    payload: EntryPayload::Membership(Membership::new(vec![btreeset! {2,3}], None)),
                }],
                leader_commit: Some(LogId::new(LeaderId::new(0, 0), 0)),
            };
            router.connect(1, None).await.send_append_entries(req).await?;

            tracing::info!("--- check that learner membership is affected");
            {
                let m = StorageHelper::new(&mut sto).get_membership().await?;

                println!("{:?}", m);
                assert_eq!(&EffectiveMembership::default(), m.committed.as_ref());
                assert_eq!(Membership::new(vec![btreeset! {2,3}], None), m.effective.membership);
            }
        }

        tracing::info!("--- add learner to the cluster to receive snapshot, which overrides the learner storage");
        {
            router.add_learner(0, 1).await.expect("failed to add new node as learner");
            log_index += 1;

            tracing::info!("--- DONE add learner");

            router.wait_for_log(&btreeset![0, 1], Some(log_index), timeout(), "add learner").await?;
            router
                .wait_for_snapshot(&btreeset![1], LogId::new(LeaderId::new(1, 0), log_index), timeout(), "")
                .await?;

            let expected_snap = Some((log_index.into(), 1));

            router
                .assert_storage_state(
                    1,
                    log_index,
                    None, /* learner does not vote */
                    LogId::new(LeaderId::new(1, 0), log_index),
                    expected_snap,
                )
                .await?;

            let m = StorageHelper::new(&mut sto).get_membership().await?;

            assert_eq!(
                Membership::new(vec![btreeset! {0}], Some(btreeset! {1})),
                m.committed.membership,
                "membership should be overridden by the snapshot"
            );
            assert_eq!(
                Membership::new(vec![btreeset! {0}], Some(btreeset! {1})),
                m.effective.membership,
                "membership should be overridden by the snapshot"
            );
        }
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(5000))
}
