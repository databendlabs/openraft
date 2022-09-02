use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::error::RemoveLearnerError;
use openraft::Config;

use crate::fixtures::RaftRouter;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remove_learner() -> Result<()> {
    // - When learner is removed, logs should stop replicating to it.

    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    let config = Arc::new(
        Config {
            replication_lag_threshold: 0,
            max_applied_log_to_keep: 2000, // prevent snapshot
            ..Default::default()
        }
        .validate()?,
    );
    let router = Arc::new(RaftRouter::new(config.clone()));

    let mut log_index = router.new_nodes_from_single(btreeset! {0}, btreeset! {1}).await?;
    let leader = router.get_raft_handle(&0).await?;

    tracing::info!("--- sends log to leader and replicates to learner");
    {
        router.client_request_many(0, "foo", 10).await;
        log_index += 10;

        for node_id in [0, 1] {
            router.wait(&node_id, timeout()).await?.log(Some(log_index), "before removing learner").await?;
        }
    }

    let learner_log_index = log_index;

    tracing::info!("--- remove learner");
    {
        leader.remove_learner(1).await?;
    }

    tracing::info!("--- sends log to leader and should not replicate to learner");
    {
        router.client_request_many(0, "foo", 10).await;
        log_index += 10;

        router.wait(&0, timeout()).await?.log(Some(log_index), "leader log after removing learner").await?;
        router
            .wait(&1, timeout())
            .await?
            .log(Some(learner_log_index), "learner log after removing learner")
            .await?;
    }

    tracing::info!("--- remove non-learner");
    {
        let res = leader.remove_learner(0).await;
        assert_eq!(Err(RemoveLearnerError::NotLearner(0)), res);

        let res = leader.remove_learner(2).await;
        assert_eq!(Err(RemoveLearnerError::NotExists(2)), res);
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
