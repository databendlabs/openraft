mod fixtures;

use std::sync::Arc;

use anyhow::Result;
use async_raft::raft::MembershipConfig;
use async_raft::Config;
use async_raft::LogId;
use async_raft::RaftStorage;
use async_raft::SnapshotPolicy;
use async_raft::State;
use fixtures::RaftRouter;
use maplit::hashset;

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
    fixtures::init_tracing();

    let snapshot_threshold: u64 = 10;

    let config = Arc::new(
        Config::build("test".into())
            .snapshot_policy(SnapshotPolicy::LogsSinceLast(snapshot_threshold))
            .validate()
            .expect("failed to build Raft config"),
    );
    let router = Arc::new(RaftRouter::new(config.clone()));

    let mut want = 0;

    tracing::info!("--- initializing cluster of 2");
    {
        router.new_raft_node(0).await;

        router.wait_for_log(&hashset![0], want, None, "empty").await?;
        router.wait_for_state(&hashset![0], State::NonVoter, None, "empty").await?;
        router.initialize_from_single_node(0).await?;
        want += 1;

        router.wait_for_log(&hashset![0], want, None, "init").await?;
        router.wait_for_state(&hashset![0], State::Leader, None, "empty").await?;

        router.new_raft_node(1).await;
        router.add_non_voter(0, 1).await?;

        router.change_membership(0, hashset![0, 1]).await?;
        want += 2;

        router.wait_for_log(&hashset![0, 1], want, None, "cluster of 2").await?;
    }

    let sto = router.get_storage_handle(&0).await?;

    tracing::info!("--- send just enough logs to trigger snapshot");
    {
        router.client_request_many(0, "0", (snapshot_threshold - want) as usize).await;
        want = snapshot_threshold;

        router.wait_for_log(&hashset![0, 1], want, None, "send log to trigger snapshot").await?;
        router.wait_for_snapshot(&hashset![0], LogId { term: 1, index: want }, None, "snapshot").await?;

        let m = sto.get_membership_config().await?;
        assert_eq!(
            MembershipConfig {
                members: hashset![0, 1],
                members_after_consensus: None
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
        //             members: hashset![0, 1],
        //             members_after_consensus: None,
        //         })),
        //     )
        //     .await;
    }

    tracing::info!("--- send just enough logs to trigger the 2nd snapshot");
    {
        router.client_request_many(0, "0", (snapshot_threshold * 2 - want) as usize).await;
        want = snapshot_threshold * 2;

        router.wait_for_log(&hashset![0, 1], want, None, "send log to trigger snapshot").await?;
        router.wait_for_snapshot(&hashset![0], LogId { term: 1, index: want }, None, "snapshot").await?;
    }

    tracing::info!("--- check membership");
    {
        let m = sto.get_membership_config().await?;
        assert_eq!(
            MembershipConfig {
                members: hashset![0, 1],
                members_after_consensus: None
            },
            m,
            "membership "
        );
    }

    Ok(())
}
