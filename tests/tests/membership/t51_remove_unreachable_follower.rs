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
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());
    router.new_raft_node(0).await;

    let mut log_index = router.new_cluster(btreeset! {0,1,2,3,4}, btreeset! {}).await?;

    tracing::info!(log_index, "--- isolate node 4");
    {
        router.set_network_error(4, true);
    }

    // logs on node 4 will stop here:
    let node4_log_index = log_index;

    tracing::info!(log_index, "--- changing config to 0,1,2");
    {
        let node = router.get_raft_handle(&0)?;
        node.change_membership([0, 1, 2], false).await?;
        log_index += 2;

        for i in &[0, 1, 2] {
            router
                .wait(i, timeout())
                .metrics(
                    |x| x.last_log_index >= Some(log_index),
                    "0,1,2 recv 2 change-membership logs",
                )
                .await?;
        }

        router
            .wait(&3, timeout())
            .metrics(
                |x| x.last_log_index >= Some(log_index - 1),
                "node-3 recv at least 1 change-membership log",
            )
            .await?;
    }

    tracing::info!(log_index, "--- replication to node 4 will be removed");
    {
        router
            .wait(&0, timeout())
            .metrics(
                |x| x.replication.as_ref().map(|y| y.contains_key(&4)) == Some(false),
                "stopped replication to node 4",
            )
            .await?;
    }

    tracing::info!(
        log_index,
        "--- restore network isolation, node 4 won't catch up log and will enter candidate state"
    );
    {
        router.set_network_error(4, false);

        router
            .wait(&4, timeout())
            .metrics(
                |x| {
                    x.last_log_index == Some(node4_log_index)
                        && (x.state == ServerState::Candidate || x.state == ServerState::Follower)
                },
                "node 4 stopped recv log and start to elect",
            )
            .await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1000))
}
