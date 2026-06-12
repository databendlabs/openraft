use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use openraft::Config;
use openraft::ServerState;
use openraft::Vote;

use crate::fixtures::MemLogStore;
use crate::fixtures::MemRaft;
use crate::fixtures::MemStateMachine;
use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// With `enable_leader_restore=false`, a restarted leader must not restore leadership
/// from the persisted committed vote. It restarts as a follower and re-acquires
/// leadership through a normal election at a higher term.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn leader_restart_leader_restore_disabled() -> anyhow::Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            enable_leader_restore: Some(false),
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- bring up cluster of 1 node");
    let mut log_index = router.new_cluster(btreeset! {0}, btreeset! {}).await?;

    tracing::info!(log_index, "--- write 1 log");
    {
        log_index += router.client_request_many(0, "foo", 1).await?;
    }

    tracing::info!(log_index, "--- stop and restart node-0");
    {
        let (node, ls, sm): (MemRaft, MemLogStore, MemStateMachine) = router.remove_node(0).unwrap();
        node.shutdown().await?;

        router.new_raft_node_with_sto(0, ls, sm).await;
    }

    tracing::info!(log_index, "--- node-0 must re-elect instead of restoring leadership");
    {
        router.wait(&0, timeout()).state(ServerState::Leader, "leader again via election").await?;

        // Term 2 proves a real election happened: restoring leadership would have
        // kept the committed vote at term 1.
        router.wait(&0, timeout()).vote(Vote::new_committed(2, 0), "vote moved to term 2").await?;

        // The new leader appends a blank log entry for its term.
        log_index += 1;
        router.wait(&0, timeout()).applied_index(Some(log_index), "logs re-applied, plus the noop").await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
