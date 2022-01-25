use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::LogId;
use openraft::Membership;
use openraft::RaftStorage;
use openraft::SnapshotPolicy;

use crate::fixtures::RaftRouter;

/// Test a second compaction should not lose membership.
/// To ensure the bug is fixed:
/// - Snapshot stores membership when compaction.
/// - But compaction does not extract membership config from Snapshot entry, only from MembershipConfig entry.
///
/// What does this test do?
///
/// - build a cluster of 2 nodes.
/// - send enough requests to trigger a snapshot.
/// - send just enough request to trigger another snapshot.
/// - ensure membership is still valid.

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn snapshot_uses_prev_snap_membership() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    let snapshot_threshold: u64 = 10;

    let config = Arc::new(
        Config {
            snapshot_policy: SnapshotPolicy::LogsSinceLast(snapshot_threshold),
            // Use 3, with 1 it triggers a compaction when replicating ent-1,
            // because ent-0 is removed.
            max_applied_log_to_keep: 3,
            ..Default::default()
        }
        .validate()?,
    );
    let router = Arc::new(RaftRouter::new(config.clone()));

    let mut log_index = router.new_nodes_from_single(btreeset! {0,1}, btreeset! {}).await?;

    let sto0 = router.get_storage_handle(&0).await?;

    tracing::info!("--- send just enough logs to trigger snapshot");
    {
        router.client_request_many(0, "0", (snapshot_threshold - 1 - log_index) as usize).await;
        log_index = snapshot_threshold - 1;

        router
            .wait_for_log(
                &btreeset![0, 1],
                Some(log_index),
                timeout(),
                "send log to trigger snapshot",
            )
            .await?;
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

        {
            let logs = sto0.get_log_entries(..).await?;
            assert_eq!(3, logs.len(), "only one applied log is kept");
        }
        let m = sto0.get_membership().await?;

        let m = m.unwrap();

        assert_eq!(Membership::new_single(btreeset! {0,1}), m.membership, "membership ");

        // TODO(xp): this assertion fails because when change-membership, a append-entries request does not update
        //           voted_for and does not call save_vote.
        //           Thus the storage layer does not know about the leader==Some(0).
        //           Update voted_for whenever a new leader is seen would solve this issue.
        // router
        //     .assert_storage_state(
        //         1,
        //         want,
        //         Some(0),
        //         want,
        //         Some((want.into(), 1, MembershipConfig {
        //             members: btreeset![0, 1],
        //             members_after_consensus: None,
        //         })),
        //     )
        //     .await;
    }

    tracing::info!("--- send just enough logs to trigger the 2nd snapshot");
    {
        router.client_request_many(0, "0", (snapshot_threshold * 2 - 1 - log_index) as usize).await;
        log_index = snapshot_threshold * 2 - 1;

        router.wait_for_log(&btreeset![0, 1], Some(log_index), None, "send log to trigger snapshot").await?;
        router
            .wait_for_snapshot(
                &btreeset![0],
                LogId {
                    term: 1,
                    index: log_index,
                },
                None,
                "snapshot",
            )
            .await?;
    }

    tracing::info!("--- check membership");
    {
        {
            let logs = sto0.get_log_entries(..).await?;
            assert_eq!(3, logs.len(), "only one applied log");
        }
        let m = sto0.get_membership().await?;

        let m = m.unwrap();

        assert_eq!(Membership::new_single(btreeset! {0,1}), m.membership, "membership ");
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(500))
}
