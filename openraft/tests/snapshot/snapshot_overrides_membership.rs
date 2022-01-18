use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::Entry;
use openraft::raft::EntryPayload;
use openraft::Config;
use openraft::LogId;
use openraft::Membership;
use openraft::RaftNetwork;
use openraft::RaftStorage;
use openraft::SnapshotPolicy;

use crate::fixtures::blank;
use crate::fixtures::RaftRouter;

/// Test membership info is sync correctly along with snapshot.
///
/// What does this test do?
///
/// - build a stable single node cluster.
/// - send enough requests to the node that log compaction will be triggered.
/// - ensure that snapshot overrides the existent membership on the learner.
#[tokio::test(flavor = "multi_thread", worker_threads = 10)]
async fn snapshot_overrides_membership() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    let snapshot_threshold: u64 = 10;

    let config = Arc::new(
        Config {
            snapshot_policy: SnapshotPolicy::LogsSinceLast(snapshot_threshold),
            max_applied_log_to_keep: 0,
            ..Default::default()
        }
        .validate()?,
    );
    let router = Arc::new(RaftRouter::new(config.clone()));

    let mut log_index = router.new_nodes_from_single(btreeset! {0}, btreeset! {}).await?;

    tracing::info!("--- send just enough logs to trigger snapshot");
    {
        router.client_request_many(0, "0", (snapshot_threshold - 1 - log_index) as usize).await;
        log_index = snapshot_threshold - 1;

        router
            .wait_for_log(
                &btreeset![0],
                Some(log_index),
                timeout(),
                "send log to trigger snapshot",
            )
            .await?;
        router.assert_stable_cluster(Some(1), Some(log_index)).await;

        router
            .wait_for_snapshot(
                &btreeset![0],
                LogId {
                    term: 1,
                    index: log_index,
                },
                timeout(),
                "snapshot",
            )
            .await?;
        router
            .assert_storage_state(
                1,
                log_index,
                Some(0),
                LogId {
                    term: 1,
                    index: log_index,
                },
                Some((log_index.into(), 1)),
            )
            .await?;
    }

    tracing::info!("--- create learner");
    {
        tracing::info!("--- create learner");
        router.new_raft_node(1).await;
        let sto = router.get_storage_handle(&1).await?;

        tracing::info!("--- add a membership config log to the learner");
        {
            let req = AppendEntriesRequest {
                term: 1,
                leader_id: 0,
                prev_log_id: None,
                entries: vec![blank(0, 0), Entry {
                    log_id: LogId { term: 1, index: 1 },
                    payload: EntryPayload::Membership(Membership::new_single(btreeset! {2,3})),
                }],
                leader_commit: Some(LogId::new(0, 0)),
            };
            router.send_append_entries(1, req).await?;

            tracing::info!("--- check that learner membership is affected");
            {
                let m = sto.get_membership().await?;

                let m = m.unwrap();

                assert_eq!(Membership::new_single(btreeset! {2,3}), m.membership);
            }
        }

        tracing::info!("--- add learner to the cluster to receive snapshot, which overrides the learner storage");
        {
            router.add_learner(0, 1).await.expect("failed to add new node as learner");
            log_index += 1;

            tracing::info!("--- DONE add learner");

            router.wait_for_log(&btreeset![0], Some(log_index), timeout(), "add learner").await?;
            router.wait_for_log(&btreeset![1], Some(log_index - 1), timeout(), "add learner").await?;
            router
                .wait_for_snapshot(
                    &btreeset![1],
                    LogId {
                        term: 1,
                        index: log_index - 1,
                    },
                    timeout(),
                    "",
                )
                .await?;

            router
                .assert_storage_state_in_node(
                    0,
                    1,
                    log_index,
                    Some(0),
                    LogId {
                        term: 1,
                        index: log_index,
                    },
                    Some(((log_index - 1).into(), 1)),
                )
                .await?;

            router
                .assert_storage_state_in_node(
                    1,
                    1,
                    log_index - 1,
                    Some(0),
                    LogId {
                        term: 1,
                        index: log_index - 1,
                    },
                    Some(((log_index - 1).into(), 1)),
                )
                .await?;
            /*
                        let expected_snap = Some((log_index.into(), 1));

                        router
                            .assert_storage_state(
                                1,
                                log_index,
                                None, /* learner does not vote */
                                LogId {
                                    term: 1,
                                    index: log_index,
                                },
                                expected_snap,
                            )
                            .await?;
            */
            let m = sto.get_membership().await?;

            let m = m.unwrap();

            assert_eq!(
                Membership::new_single(btreeset! {0}),
                m.membership,
                "membership should be overridden by the snapshot"
            );
        }
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(5000))
}
