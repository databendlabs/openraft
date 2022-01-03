use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::raft::AddLearnerResponse;
use openraft::Config;
use openraft::LogId;
use openraft::RaftStorage;

use crate::fixtures::RaftRouter;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn add_learner_basic() -> Result<()> {
    //
    // - Add leader, expect NoChange
    // - Add a non-voter, expect raft to block until catching up.
    // - Re-add should fail.

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

    let mut n_logs = router.new_nodes_from_single(btreeset! {0}, btreeset! {}).await?;

    tracing::info!("--- re-adding leader does nothing");
    {
        let res = router.add_learner(0, 0).await?;
        assert_eq!(
            AddLearnerResponse {
                matched: LogId::new(1, n_logs)
            },
            res
        );
    }

    tracing::info!("--- add new node node-1");
    {
        tracing::info!("--- write up to 1000 logs");

        router.client_request_many(0, "learner_add", 1000 - n_logs as usize).await;
        n_logs = 1000;

        tracing::info!("--- write up to 1000 logs done");

        router.wait_for_log(&btreeset! {0}, n_logs, timeout(), "write 1000 logs to leader").await?;

        router.new_raft_node(1).await;
        router.add_learner(0, 1).await?;

        tracing::info!("--- add_learner blocks until the replication catches up");
        let sto1 = router.get_storage_handle(&1).await?;

        let logs = sto1.get_log_entries(..).await?;

        assert_eq!(n_logs, logs[logs.len() - 1].log_id.index);
        // 0-th log
        assert_eq!(n_logs + 1, logs.len() as u64);

        router.wait_for_log(&btreeset! {0,1}, n_logs, timeout(), "replication to learner").await?;
    }

    tracing::info!("--- re-add node-1, expect error");
    {
        let res = router.add_learner(0, 1).await?;
        assert_eq!(
            AddLearnerResponse {
                matched: LogId::new(1, n_logs)
            },
            res
        );
    }

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn add_learner_non_blocking() -> Result<()> {
    //
    // - Add leader, expect NoChange
    // - Add a non-voter, expect raft to block until catching up.
    // - Re-add should fail.

    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    let config = Arc::new(
        Config {
            replication_lag_threshold: 0,
            ..Default::default()
        }
        .validate()?,
    );
    let router = Arc::new(RaftRouter::new(config.clone()));

    let mut n_logs = router.new_nodes_from_single(btreeset! {0}, btreeset! {}).await?;

    tracing::info!("--- add new node node-1, in non blocking mode");
    {
        tracing::info!("--- write up to 100 logs");

        router.client_request_many(0, "learner_add", 100 - n_logs as usize).await;
        n_logs = 100;

        router.wait(&0, timeout()).await?.log(n_logs, "received 100 logs").await?;

        router.new_raft_node(1).await;
        let res = router.add_learner_with_blocking(0, 1, false).await?;

        assert_eq!(
            AddLearnerResponse {
                matched: LogId::new(0, 0)
            },
            res
        );
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_micros(500))
}
