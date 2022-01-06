use std::sync::Arc;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::State;

use crate::fixtures::RaftRouter;

/// Current leader tests.
///
/// What does this test do?
///
/// - create a stable 3-node cluster.
/// - call the current_leader interface on the all nodes, and assert success.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn current_leader() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    // Setup test dependencies.
    let config = Arc::new(Config::default().validate()?);
    let router = Arc::new(RaftRouter::new(config.clone()));
    router.new_raft_node(0).await;
    router.new_raft_node(1).await;
    router.new_raft_node(2).await;

    let mut n_logs = 0;

    // Assert all nodes are in learner state & have no entries.
    router.wait_for_log(&btreeset![0, 1, 2], n_logs, None, "empty").await?;
    router.wait_for_state(&btreeset![0, 1, 2], State::Learner, None, "empty").await?;
    router.assert_pristine_cluster().await;

    // Initialize the cluster, then assert that a stable cluster was formed & held.
    tracing::info!("--- initializing cluster");
    router.initialize_from_single_node(0).await?;
    n_logs += 1;

    router.wait_for_log(&btreeset![0, 1, 2], n_logs, None, "init").await?;
    router.assert_stable_cluster(Some(1), Some(n_logs)).await;

    // Get the ID of the leader, and assert that current_leader succeeds.
    let leader = router.leader().await.expect("leader not found");
    assert_eq!(leader, 0, "expected leader to be node 0, got {}", leader);

    for i in 0..3 {
        let leader = router.current_leader(i).await;
        assert_eq!(leader, Some(0), "expected leader to be node 0, got {:?}", leader);
    }

    Ok(())
}
