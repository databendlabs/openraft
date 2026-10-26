use std::sync::Arc;

use maplit::btreeset;
use openraft::Config;
use openraft::ServerState;
use openraft::Vote;

use crate::fixtures::Node;
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
        let written = router.client_request_many(0, "foo", 1).await?;
        log_index = written.last_log_id.expect("nonempty workload").index();
    }

    tracing::info!(log_index, "--- stop and restart node-0");
    {
        let Node {
            log_store: ls,
            state_machine: sm,
            ..
        } = router.stop_node(0).await?;

        router.new_raft_node_with_sto(0, ls, sm).await?;
    }

    tracing::info!(log_index, "--- node-0 must re-elect instead of restoring leadership");
    {
        router.wait(&0).state(ServerState::Leader, "leader again via election").await?;

        // Term 2 proves a real election happened: restoring leadership would have
        // kept the committed vote at term 1.
        router.wait(&0).vote(Vote::new_committed(2, 0), "vote moved to term 2").await?;

        // The new leader appends a blank log entry for its term.
        log_index += 1;
        router.wait(&0).applied_index(Some(log_index), "logs re-applied, plus the noop").await?;
    }

    Ok(())
}
