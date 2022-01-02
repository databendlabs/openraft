use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_raft::Config;
use maplit::btreeset;

use crate::fixtures::RaftRouter;

/// Replication should stop after a follower is removed from membership.
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn stop_replication_to_removed_follower() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    // Setup test dependencies.
    let config = Arc::new(Config::default().validate()?);
    let router = Arc::new(RaftRouter::new(config.clone()));
    router.new_raft_node(0).await;

    let mut n_logs = router.new_nodes_from_single(btreeset! {0,1,2}, btreeset! {}).await?;

    tracing::info!("--- add node 3,4");

    router.new_raft_node(3).await;
    router.new_raft_node(4).await;

    router.add_learner(0, 3).await?;
    router.add_learner(0, 4).await?;

    tracing::info!("--- changing config to 2,3,4");
    {
        router.change_membership(0, btreeset![0, 3, 4]).await?;
        n_logs += 2;

        for i in 0..5 {
            router
                .wait(&i, timeout())
                .await?
                .metrics(|x| x.last_log_index >= n_logs, "all nodes recv change-membership logs")
                .await?;
        }
    }

    tracing::info!("--- write to new cluster, cuurent log={}", n_logs);
    {
        let n = 10;
        router.client_request_many(0, "after_change", n).await;
        n_logs += n as u64;

        for i in &[0, 3, 4] {
            router
                .wait(i, timeout())
                .await?
                .metrics(|x| x.last_applied >= n_logs, "new cluster recv new logs")
                .await?;
        }
    }

    for i in &[1, 2] {
        router
            .wait(i, timeout())
            .await?
            .metrics(|x| x.last_applied < n_logs, "old cluster does not recv new logs")
            .await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(5000))
}
