use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use fixtures::RaftRouter;
use maplit::btreeset;
use openraft::Config;
use openraft::State;

#[macro_use]
mod fixtures;

/// When a node is removed from cluster, replication to it should be stopped.
///
/// - brings 5 nodes online: one leader and 4 follower.
/// - asserts that the leader was able to successfully commit logs and that the followers has successfully replicated
///   the payload.
/// - remove one follower: node-4
/// - asserts node-4 becomes learner and the leader stops sending logs to it.
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn add_remove_voter() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    let cluster_of_5 = btreeset![0, 1, 2, 3, 4];
    let cluster_of_4 = btreeset![0, 1, 2, 3];

    let config = Arc::new(Config::default().validate()?);
    let router = Arc::new(RaftRouter::new(config.clone()));

    let mut n_logs = router.new_nodes_from_single(cluster_of_5.clone(), btreeset! {}).await?;

    tracing::info!("--- write 100 logs");
    {
        router.client_request_many(0, "client", 100).await;
        n_logs += 100;

        router.wait_for_log(&cluster_of_5, n_logs, timeout(), "write 100 logs").await?;
    }

    tracing::info!("--- remove n{}", 4);
    {
        router.change_membership(0, cluster_of_4.clone()).await?;
        n_logs += 2; // two member-change logs

        router.wait_for_log(&cluster_of_4, n_logs, timeout(), "removed node-4").await?;
        router.wait(&4, timeout()).await?.state(State::Learner, "").await?;
    }

    tracing::info!("--- write another 100 logs");
    {
        router.client_request_many(0, "client", 100).await;
        n_logs += 100;
    }

    router.wait_for_log(&cluster_of_4, n_logs, timeout(), "4 nodes recv logs 100~200").await?;

    tracing::info!("--- log will not be sync to removed node");
    {
        let x = router.latest_metrics().await;
        assert!(x[4].last_log_index < n_logs - 50);
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1000))
}
