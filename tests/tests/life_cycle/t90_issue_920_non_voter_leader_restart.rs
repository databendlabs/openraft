use std::sync::Arc;
use std::time::Duration;

use openraft::storage::RaftLogStorage;
use openraft::Config;
use openraft::ServerState;
use openraft::Vote;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Special case: A leader that is not a member(neither a voter or non-voter) should be started too,
/// as a learner.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn issue_920_non_member_leader_restart() -> anyhow::Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    let (mut log_store, sm) = router.new_store();
    // Set committed vote that believes node 0 is the leader.
    log_store.save_vote(&Vote::new_committed(1, 0)).await?;
    router.new_raft_node_with_sto(0, log_store, sm).await;

    router
        .wait(&0, timeout())
        .state(ServerState::Learner, "node 0 becomes learner when startup")
        .await?;

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1000))
}
