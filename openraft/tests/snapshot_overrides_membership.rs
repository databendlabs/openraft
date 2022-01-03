use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use fixtures::RaftRouter;
use maplit::btreeset;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::Entry;
use openraft::raft::EntryPayload;
use openraft::raft::Membership;
use openraft::Config;
use openraft::LogId;
use openraft::RaftNetwork;
use openraft::RaftStorage;
use openraft::SnapshotPolicy;
use openraft::State;

#[macro_use]
mod fixtures;

/// Test membership info is sync correctly along with snapshot.
///
/// What does this test do?
///
/// - build a stable single node cluster.
/// - send enough requests to the node that log compaction will be triggered.
/// - ensure that snapshot overrides the existent membership on the non-voter.
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

    let mut want = 0;

    tracing::info!("--- initializing cluster");
    {
        router.new_raft_node(0).await;

        router.wait_for_log(&btreeset![0], want, timeout(), "empty").await?;
        router.wait_for_state(&btreeset![0], State::Learner, timeout(), "empty").await?;
        router.initialize_from_single_node(0).await?;
        want += 1;

        router.wait_for_log(&btreeset![0], want, timeout(), "init leader").await?;
        router.assert_stable_cluster(Some(1), Some(want)).await;
    }

    tracing::info!("--- send just enough logs to trigger snapshot");
    {
        router.client_request_many(0, "0", (snapshot_threshold - want) as usize).await;
        want = snapshot_threshold;

        router.wait_for_log(&btreeset![0], want, timeout(), "send log to trigger snapshot").await?;
        router.assert_stable_cluster(Some(1), Some(want)).await;

        router
            .wait_for_snapshot(&btreeset![0], LogId { term: 1, index: want }, timeout(), "snapshot")
            .await?;
        router
            .assert_storage_state(1, want, Some(0), LogId { term: 1, index: want }, Some((want.into(), 1)))
            .await?;
    }

    tracing::info!("--- create non-voter");
    {
        tracing::info!("--- create non-voter");
        router.new_raft_node(1).await;
        let sto = router.get_storage_handle(&1).await?;

        tracing::info!("--- add a membership config log to the non-voter");
        {
            let req = AppendEntriesRequest {
                term: 1,
                leader_id: 0,
                prev_log_id: LogId::new(0, 0),
                entries: vec![Entry {
                    log_id: LogId { term: 1, index: 1 },
                    payload: EntryPayload::Membership(Membership::new_single(btreeset! {2,3})),
                }],
                leader_commit: LogId::new(0, 0),
            };
            router.send_append_entries(1, req).await?;

            tracing::info!("--- check that non-voter membership is affected");
            {
                let m = sto.get_membership_config().await?;
                assert_eq!(Membership::new_single(btreeset! {2,3}), m.membership);
            }
        }

        tracing::info!("--- add non-voter to the cluster to receive snapshot, which overrides the non-voter storage");
        {
            router.add_learner(0, 1).await.expect("failed to add new node as non-voter");

            tracing::info!("--- DONE add non-voter");

            router.wait_for_log(&btreeset![0, 1], want, timeout(), "add non-voter").await?;
            router.wait_for_snapshot(&btreeset![1], LogId { term: 1, index: want }, timeout(), "").await?;

            let expected_snap = Some((want.into(), 1));

            router
                .assert_storage_state(
                    1,
                    want,
                    None, /* non-voter does not vote */
                    LogId { term: 1, index: want },
                    expected_snap,
                )
                .await?;

            let m = sto.get_membership_config().await?;
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
