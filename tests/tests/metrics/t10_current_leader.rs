use std::sync::Arc;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// Current leader tests.
///
/// What does this test do?
///
/// - create a stable 3-node cluster.
/// - call the current_leader interface on the all nodes, and assert success.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn current_leader() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    let _log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    // Get the ID of the leader, and assert that current_leader succeeds.
    let leader = router.leader().expect("leader not found");
    assert_eq!(leader, 0, "expected leader to be node 0, got {}", leader);

    for i in 0..3 {
        let leader = router.current_leader(i).await;
        assert_eq!(leader, Some(0), "expected leader to be node 0, got {:?}", leader);
    }

    Ok(())
}
