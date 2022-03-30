use std::sync::Arc;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Current leader tests.
///
/// What does this test do?
///
/// - create a stable 3-node cluster.
/// - call the current_leader interface on the all nodes, and assert success.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn current_leader() -> Result<()> {
    // Setup test dependencies.
    let config = Arc::new(Config::default().validate()?);
    let mut router = RaftRouter::new(config.clone());

    let _log_index = router.new_nodes_from_single(btreeset! {0,1,2}, btreeset! {}).await?;

    // Get the ID of the leader, and assert that current_leader succeeds.
    let leader = router.leader().expect("leader not found");
    assert_eq!(leader, 0, "expected leader to be node 0, got {}", leader);

    for i in 0..3 {
        let leader = router.current_leader(i).await;
        assert_eq!(leader, Some(0), "expected leader to be node 0, got {:?}", leader);
    }

    Ok(())
}
