use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::ServerState;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Replication should stop after a **unreachable** follower is removed from membership.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn stop_replication_to_removed_unreachable_follower_network_failure() -> Result<()> {
    // If the uniform membership is committed and replication to a node encountered 2 network failure, just remove it.
    let config = Arc::new(Config::build(&["foo", "--remove-replication=max_network_failures:2"])?);

    let mut router = RaftRouter::new(config.clone());
    router.new_raft_node(0).await;

    let mut log_index = router.new_nodes_from_single(btreeset! {0,1,2,3,4}, btreeset! {}).await?;

    router.wait_for_log(&btreeset![0, 1, 2, 3, 4], Some(log_index), timeout(), "cluster of 5").await?;

    tracing::info!("--- isolate node 4");
    {
        router.isolate_node(4).await;
    }

    // logs on node 4 will stop here:
    let node4_log_index = log_index;

    tracing::info!("--- changing config to 0,1,2");
    {
        let node = router.get_raft_handle(&0)?;
        node.change_membership(btreeset![0, 1, 2], true, false).await?;
        log_index += 2;

        for i in &[0, 1, 2, 3] {
            router
                .wait(i, timeout())
                .metrics(
                    |x| x.last_log_index >= Some(log_index),
                    "0,1,2,3 recv change-membership logs",
                )
                .await?;
        }
    }

    tracing::info!("--- replication to node 4 will be removed");
    {
        router
            .wait(&0, timeout())
            .metrics(
                |x| x.replication.as_ref().map(|y| y.data().replication.contains_key(&4)) == Some(false),
                "stopped replication to node 4",
            )
            .await?;
    }

    tracing::info!("--- restore network isolation, node 4 won't catch up log and will enter candidate state");
    {
        router.restore_node(4).await;

        router
            .wait(&4, timeout())
            .metrics(
                |x| x.last_log_index == Some(node4_log_index) && x.state == ServerState::Candidate,
                "node 4 stopped recv log and start to elect",
            )
            .await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1000))
}
