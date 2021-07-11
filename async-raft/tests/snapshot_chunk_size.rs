mod fixtures;

use std::sync::Arc;

use anyhow::Result;
use async_raft::raft::MembershipConfig;
use async_raft::Config;
use async_raft::LogId;
use async_raft::SnapshotPolicy;
use async_raft::State;
use fixtures::RaftRouter;
use maplit::hashset;

/// Test transfer snapshot in small chnuks
///
/// What does this test do?
///
/// - build a stable single node cluster.
/// - send enough requests to the node that log compaction will be triggered.
/// - add non-voter and assert that they receive the snapshot and logs.
///
/// export RUST_LOG=async_raft,memstore,snapshot_chunk_size=trace
/// cargo test -p async-raft --test snapshot_chunk_size
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn snapshot_chunk_size() -> Result<()> {
    fixtures::init_tracing();

    let snapshot_threshold: u64 = 10;

    let config = Arc::new(
        Config::build("test".into())
            .snapshot_policy(SnapshotPolicy::LogsSinceLast(snapshot_threshold))
            .snapshot_max_chunk_size(10)
            .validate()
            .expect("failed to build Raft config"),
    );
    let router = Arc::new(RaftRouter::new(config.clone()));

    let mut want = 0;

    tracing::info!("--- initializing cluster");
    {
        router.new_raft_node(0).await;

        router.wait_for_log(&hashset![0], want, None, "empty").await?;
        router.wait_for_state(&hashset![0], State::NonVoter, None, "empty").await?;

        router.initialize_from_single_node(0).await?;
        want += 1;

        router.wait_for_log(&hashset![0], want, None, "init leader").await?;
    }

    tracing::info!("--- send just enough logs to trigger snapshot");
    {
        router.client_request_many(0, "0", (snapshot_threshold - want) as usize).await;
        want = snapshot_threshold;

        let want_snap = Some((want.into(), 1, MembershipConfig {
            members: hashset![0u64],
            members_after_consensus: None,
        }));

        router.wait_for_log(&hashset![0], want, None, "send log to trigger snapshot").await?;
        router.wait_for_snapshot(&hashset![0], LogId { term: 1, index: want }, None, "snapshot").await?;
        router.assert_storage_state(1, want, Some(0), want, want_snap).await;
    }

    tracing::info!("--- add non-voter to receive snapshot and logs");
    {
        router.new_raft_node(1).await;
        router.add_non_voter(0, 1).await.expect("failed to add new node as non-voter");

        let want_snap = Some((want.into(), 1, MembershipConfig {
            members: hashset![0u64],
            members_after_consensus: None,
        }));

        router.wait_for_log(&hashset![0, 1], want, None, "add non-voter").await?;
        router.wait_for_snapshot(&hashset![1], LogId { term: 1, index: want }, None, "").await?;
        router.assert_storage_state(1, want, None /* non-voter does not vote */, want, want_snap).await;
    }

    Ok(())
}
