use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_raft::raft::MembershipConfig;
use async_raft::Config;
use async_raft::LogId;
use async_raft::RaftStorage;
use async_raft::SnapshotPolicy;
use async_raft::State;
use fixtures::RaftRouter;
use maplit::btreeset;

#[macro_use]
mod fixtures;

/// Test a second compaction should not lose membership.
///
/// What does this test do?
///
/// - build a cluster of 2 nodes.
/// - send enough requests to trigger a snapshot.
/// - send just enough request to trigger another snapshot.
/// - ensure membership is still valid.
///
/// export RUST_LOG=async_raft,memstore,snapshot_uses_prev_snap_membership=trace
/// cargo test -p async-raft --test snapshot_uses_prev_snap_membership
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn snapshot_uses_prev_snap_membership() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    let snapshot_threshold: u64 = 10;

    let config = Arc::new(
        Config {
            snapshot_policy: SnapshotPolicy::LogsSinceLast(snapshot_threshold),
            ..Default::default()
        }
        .validate()?,
    );
    let router = Arc::new(RaftRouter::new(config.clone()));

    let mut want = 0;

    tracing::info!("--- initializing cluster of 2");
    {
        router.new_raft_node(0).await;

        router.wait_for_log(&btreeset![0], want, None, "empty").await?;
        router.wait_for_state(&btreeset![0], State::NonVoter, None, "empty").await?;
        router.initialize_from_single_node(0).await?;
        want += 1;

        router.wait_for_log(&btreeset![0], want, None, "init").await?;
        router.wait_for_state(&btreeset![0], State::Leader, None, "empty").await?;

        router.new_raft_node(1).await;
        router.add_non_voter(0, 1).await?;

        router.change_membership(0, btreeset![0, 1]).await?;
        want += 2;

        router.wait_for_log(&btreeset![0, 1], want, None, "cluster of 2").await?;
    }

    let sto0 = router.get_storage_handle(&0).await?;

    tracing::info!("--- send just enough logs to trigger snapshot");
    {
        router.client_request_many(0, "0", (snapshot_threshold - want) as usize).await;
        want = snapshot_threshold;

        router.wait_for_log(&btreeset![0, 1], want, to(), "send log to trigger snapshot").await?;
        router.wait_for_snapshot(&btreeset![0], LogId { term: 1, index: want }, to(), "snapshot").await?;

        {
            let logs = sto0.get_log_entries(..).await?;
            assert_eq!(1, logs.len(), "only one snapshot pointer log");
        }
        let m = sto0.get_membership_config().await?;
        assert_eq!(
            MembershipConfig {
                members: btreeset![0, 1],
                members_after_consensus: None,
            },
            m,
            "membership "
        );

        // TODO(xp): this assertion fails because when change-membership, a append-entries request does not update
        //           voted_for and does not call save_hard_state.
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
        router.client_request_many(0, "0", (snapshot_threshold * 2 - want) as usize).await;
        want = snapshot_threshold * 2;

        router.wait_for_log(&btreeset![0, 1], want, None, "send log to trigger snapshot").await?;
        router.wait_for_snapshot(&btreeset![0], LogId { term: 1, index: want }, None, "snapshot").await?;
    }

    tracing::info!("--- check membership");
    {
        {
            let logs = sto0.get_log_entries(..).await?;
            assert_eq!(1, logs.len(), "only one snapshot pointer log");
        }
        let m = sto0.get_membership_config().await?;
        assert_eq!(
            MembershipConfig {
                members: btreeset![0, 1],
                members_after_consensus: None,
            },
            m,
            "membership "
        );
    }

    Ok(())
}

fn to() -> Option<Duration> {
    Some(Duration::from_millis(500))
}
