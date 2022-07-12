use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::LogIdOptionExt;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Replication should stop after a follower is removed from membership.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn stop_replication_to_removed_follower() -> Result<()> {
    // Setup test dependencies.
    let config = Arc::new(Config::default().validate()?);
    let mut router = RaftRouter::new(config.clone());
    router.new_raft_node(0);

    let mut log_index = router.new_nodes_from_single(btreeset! {0,1,2}, btreeset! {}).await?;

    tracing::info!("--- add node 3,4");

    router.new_raft_node(3);
    router.new_raft_node(4);

    router.add_learner(0, 3).await?;
    router.add_learner(0, 4).await?;
    log_index += 2;
    router.wait_for_log(&btreeset![0, 1, 2], Some(log_index), None, "cluster of 2 learners").await?;

    tracing::info!("--- changing config to 2,3,4");
    {
        let node = router.get_raft_handle(&0)?;
        node.change_membership(btreeset![0, 3, 4], true, false).await?;
        log_index += 2;

        for i in [0, 3, 4] {
            router
                .wait(&i, timeout())
                .metrics(
                    |x| x.last_log_index >= Some(log_index),
                    "new cluster nodes recv 2 change-membership logs",
                )
                .await?;
        }

        for i in [1, 2] {
            router
                .wait(&i, timeout())
                .metrics(
                    |x| x.last_log_index >= Some(log_index - 1),
                    "removed nodes recv at least 1 change-membership log",
                )
                .await?;
        }
    }

    tracing::info!("--- write to new cluster, cuurent log={}", log_index);
    {
        let n = 10;
        router.client_request_many(0, "after_change", n).await?;
        log_index += n as u64;

        for i in &[0, 3, 4] {
            router
                .wait(i, timeout())
                .metrics(
                    |x| x.last_applied.index() >= Some(log_index),
                    "new cluster recv new logs",
                )
                .await?;
        }
    }

    for i in &[1, 2] {
        router
            .wait(i, timeout())
            .metrics(
                |x| x.last_applied.index() < Some(log_index),
                "old cluster does not recv new logs",
            )
            .await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(5000))
}
