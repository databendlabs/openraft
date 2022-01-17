use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use fixtures::RaftRouter;
use maplit::btreeset;
use openraft::Config;
use openraft::LogIdOptionExt;
use openraft::State;
use tracing_futures::Instrument;

#[macro_use]
mod fixtures;

/// Cluster concurrent_write_and_add_learner test.
///
/// Internally when replication goes to LaggingState(a non-leader lacks a lot logs), the
/// ReplicationCore purges `outbound_buffer` and `replication_buffer` and then sends all
/// **committed** logs found in storage.
///
/// Thus if there are uncommitted logs in `replication_buffer`, these log will never have chance to
/// be replicated, even when replication goes back to LineRateState.
/// Since LineRateState only replicates logs from `ReplicationCore.outbound_buffer` and
/// `ReplicationCore.replication_buffer`.
///
/// This test ensures that when replication goes to LineRateState, it tries to re-send all logs
/// found in storage(including those that are removed from the two buffers.
///
/// NOTE: this test needs a multi nodes cluster because a single-node cluster will commit a log
/// at once.
///
///
/// What does this test do?
///
/// - brings a 3 candidates cluster online.
/// - add another learner and at the same time write a log.
/// - asserts that all of the leader, followers and the learner receives all logs.
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn concurrent_write_and_add_learner() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    let timeout = Duration::from_millis(500);
    let candidates = btreeset![0, 1, 2];

    // Setup test dependencies.
    let config = Arc::new(Config::default().validate()?);
    let router = Arc::new(RaftRouter::new(config.clone()));

    router.new_raft_node(0).await;

    let mut n_logs;

    tracing::info!("--- initializing cluster of 1 node");
    {
        router.initialize_from_single_node(0).await?;
        n_logs = 1;

        wait_log(router.clone(), &btreeset![0], n_logs).await?;
    }

    tracing::info!("--- adding two candidate nodes");
    {
        // Sync some new nodes.
        router.new_raft_node(1).await;
        router.new_raft_node(2).await;
        router.add_learner(0, 1).await?;
        router.add_learner(0, 2).await?;
        n_logs += 2; // two add_learner logs

        tracing::info!("--- changing cluster config");

        router.change_membership(0, candidates.clone()).await?;
        n_logs += 2; // Tow member change logs

        wait_log(router.clone(), &candidates, n_logs).await?;
        router.assert_stable_cluster(Some(1), Some(n_logs)).await; // Still in term 1, so leader is still node 0.
    }

    let leader = router.leader().await.unwrap();

    tracing::info!("--- write one log");
    {
        router.client_request_many(leader, "client", 1).await;
        n_logs += 1;

        wait_log(router.clone(), &candidates, n_logs).await?;
    }

    // Concurrently add Learner and write another log.
    tracing::info!("--- concurrently add learner and write another log");
    {
        router.new_raft_node(3).await;
        let r = router.clone();

        let handle = {
            tokio::spawn(
                async move {
                    r.add_learner(leader, 3).await.unwrap();
                    Ok::<(), anyhow::Error>(())
                }
                .instrument(tracing::debug_span!("spawn-add-learner")),
            )
        };
        n_logs += 1; // one add_learner log
        router.client_request_many(leader, "client", 1).await;
        n_logs += 1;

        let _ = handle.await?;
    };

    wait_log(router.clone(), &candidates, n_logs).await?;
    router
        .wait_for_metrics(
            &3u64,
            |x| x.state == State::Learner,
            Some(timeout),
            &format!("n{}.state -> {:?}", 3, State::Learner),
        )
        .await?;

    // THe learner should receive the last written log
    router
        .wait_for_metrics(
            &3u64,
            |x| x.last_log_index == Some(n_logs),
            Some(timeout),
            &format!("n{}.last_log_index -> {}", 3, n_logs),
        )
        .await?;

    Ok(())
}

async fn wait_log(
    router: std::sync::Arc<fixtures::RaftRouter>,
    node_ids: &BTreeSet<u64>,
    want_log: u64,
) -> anyhow::Result<()> {
    let timeout = Duration::from_millis(500);
    for i in node_ids.iter() {
        router
            .wait_for_metrics(
                i,
                |x| x.last_log_index == Some(want_log),
                Some(timeout),
                &format!("n{}.last_log_index -> {}", i, want_log),
            )
            .await?;
        router
            .wait_for_metrics(
                i,
                |x| x.last_applied.index() == Some(want_log),
                Some(timeout),
                &format!("n{}.last_applied -> {}", i, want_log),
            )
            .await?;
    }
    Ok(())
}
