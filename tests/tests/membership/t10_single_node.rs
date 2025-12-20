use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::RaftLogReader;
use openraft::ReadPolicy;
use openraft::Vote;
use openraft::storage::RaftLogStorage;
use openraft::storage::RaftStateMachine;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// Single-node cluster initialization test.
///
/// What does this test do?
///
/// - brings 1 node online with only knowledge of itself.
/// - asserts that it remains in learner state with no activity (it should be completely passive).
/// - initializes the cluster with membership config including just the one node.
/// - asserts that the cluster was able to come online, and that the one node became leader.
/// - asserts that the leader was able to successfully commit its initial payload.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn single_node() -> Result<()> {
    // Setup test dependencies.
    let config = Arc::new(
        Config {
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let mut log_index = router.new_cluster(btreeset! {0}, btreeset! {}).await?;

    // Write some data to the single node cluster.
    log_index += router.client_request_many(0, "0", 1000).await?;
    router.wait(&0, timeout()).applied_index(Some(log_index), "client_request_many").await?;

    let (mut sto, mut sm) = router.get_storage_handle(&0)?;
    assert_eq!(sto.get_log_state().await?.last_log_id, Some(log_id(1, 0, log_index)));
    assert_eq!(sto.read_vote().await?, Some(Vote::new_committed(1, 0)));

    let (last_applied, _) = sm.applied_state().await?;
    assert_eq!(last_applied, Some(log_id(1, 0, log_index)));

    // Read some data from the single node cluster.
    router.ensure_linearizable(0, ReadPolicy::ReadIndex).await?;

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1000))
}
