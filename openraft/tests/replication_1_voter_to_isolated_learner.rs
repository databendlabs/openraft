use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use fixtures::RaftRouter;
use maplit::btreeset;
use openraft::Config;

#[macro_use]
mod fixtures;

/// Test replication to learner that is not in membership should not block.
///
/// What does this test do?
///
/// - bring on a cluster of 1 voter and 1 learner.
/// - isolate replication to node 1.
/// - client write should not be blocked.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn replication_1_voter_to_isolated_learner() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    let config = Arc::new(Config::default().validate()?);
    let router = Arc::new(RaftRouter::new(config.clone()));

    let mut log_index = router.new_nodes_from_single(btreeset! {0}, btreeset! {1}).await?;

    tracing::info!("--- stop replication to node 1");
    {
        router.isolate_node(1).await;

        router.client_request_many(0, "0", (10 - log_index) as usize).await;
        log_index = 10;

        router
            .wait_for_log(
                &btreeset![0],
                Some(log_index),
                timeout(),
                "send log to trigger snapshot",
            )
            .await?;
    }

    tracing::info!("--- restore replication to node 1");
    {
        router.restore_node(1).await;

        router.client_request_many(0, "0", (10 - log_index) as usize).await;
        log_index = 10;

        router
            .wait_for_log(
                &btreeset![0],
                Some(log_index),
                timeout(),
                "send log to trigger snapshot",
            )
            .await?;
    }
    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1000))
}
