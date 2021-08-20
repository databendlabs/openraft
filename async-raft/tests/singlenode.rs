use std::sync::Arc;

use anyhow::Result;
use async_raft::Config;
use async_raft::LogId;
use async_raft::State;
use fixtures::RaftRouter;
use maplit::btreeset;

#[macro_use]
mod fixtures;

/// Single-node cluster initialization test.
///
/// What does this test do?
///
/// - brings 1 node online with only knowledge of itself.
/// - asserts that it remains in non-voter state with no activity (it should be completely passive).
/// - initializes the cluster with membership config including just the one node.
/// - asserts that the cluster was able to come online, and that the one node became leader.
/// - asserts that the leader was able to successfully commit its initial payload.
///
/// RUST_LOG=async_raft,memstore,singlenode=trace cargo test -p async-raft --test singlenode
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn singlenode() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    // Setup test dependencies.
    let config = Arc::new(Config::build("test".into()).validate().expect("failed to build Raft config"));
    let router = Arc::new(RaftRouter::new(config.clone()));
    router.new_raft_node(0).await;

    let mut want = 0;

    // Assert all nodes are in non-voter state & have no entries.
    router.wait_for_log(&btreeset![0], want, None, "empty").await?;
    router.wait_for_state(&btreeset![0], State::NonVoter, None, "empty").await?;
    router.assert_pristine_cluster().await;

    // Initialize the cluster, then assert that a stable cluster was formed & held.
    tracing::info!("--- initializing cluster");
    router.initialize_from_single_node(0).await?;
    want += 1;

    router.wait_for_log(&btreeset![0], want, None, "init").await?;
    router.assert_stable_cluster(Some(1), Some(1)).await;

    // Write some data to the single node cluster.
    router.client_request_many(0, "0", 1000).await;
    router.assert_stable_cluster(Some(1), Some(1001)).await;
    router.assert_storage_state(1, 1001, Some(0), LogId { term: 1, index: 1001 }, None).await;

    // Read some data from the single node cluster.
    router.client_read(0).await?;

    Ok(())
}
