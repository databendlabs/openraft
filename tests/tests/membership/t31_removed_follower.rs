use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::LogIdOptionExt;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// Replication should stop after a follower is removed from membership.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn stop_replication_to_removed_follower() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());
    router.new_raft_node(0).await;

    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    tracing::info!(log_index, "--- add node 3,4");

    router.new_raft_node(3).await;
    router.new_raft_node(4).await;

    router.add_learner(0, 3).await?;
    router.add_learner(0, 4).await?;
    log_index += 2;
    for id in [0, 1, 2] {
        router.wait(&id, None).applied_index(Some(log_index), "cluster of 2 learners").await?;
    }

    tracing::info!(log_index, "--- changing config to 0,3,4");
    {
        let node = router.get_raft_handle(&0)?;
        node.change_membership([0, 3, 4], false).await?;
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

        let res1 = router
            .wait(&1, timeout())
            .metrics(
                |x| x.last_log_index >= Some(log_index - 1),
                "removed node-1 recv at least 1 change-membership log",
            )
            .await;

        let res2 = router
            .wait(&2, timeout())
            .metrics(
                |x| x.last_log_index >= Some(log_index - 1),
                "removed node-2 recv at least 1 change-membership log",
            )
            .await;

        tracing::info!("result waiting for node-1: {:?}", res1);
        tracing::info!("result waiting for node-2: {:?}", res2);

        assert!(
            res1.is_ok() || res2.is_ok(),
            "committing the first membership log only need to replication to one of node-1 and node-2. node-1 res: {:?}; node-2 res: {:?}",
            res1,
            res2
        );
    }

    tracing::info!(log_index, "--- write to new cluster, current log={}", log_index);
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
    Some(Duration::from_millis(1000))
}
