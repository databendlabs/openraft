use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_raft::Config;
use fixtures::RaftRouter;
use maplit::btreeset;

#[macro_use]
mod fixtures;

/// Test replication to non-voter that is not in membership should not block.
///
/// What does this test do?
///
/// - bring on a cluster of 1 voter and 1 non-voter.
/// - isolate replication to node 1.
/// - client write should not be blocked.
///
/// export RUST_LOG=async_raft,memstore,replication_1_voter_to_isolated_learner=trace
/// cargo test -p async-raft --test replication_1_voter_to_isolated_learner
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn replication_1_voter_to_isolated_learner() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    let config = Arc::new(Config::default().validate()?);
    let router = Arc::new(RaftRouter::new(config.clone()));

    let mut n_logs = router.new_nodes_from_single(btreeset! {0}, btreeset! {1}).await?;

    tracing::info!("--- stop replication to node 1");
    {
        router.isolate_node(1).await;

        router.client_request_many(0, "0", (10 - n_logs) as usize).await;
        n_logs = 10;

        router.wait_for_log(&btreeset![0], n_logs, timeout(), "send log to trigger snapshot").await?;
    }

    tracing::info!("--- restore replication to node 1");
    {
        router.restore_node(1).await;

        router.client_request_many(0, "0", (10 - n_logs) as usize).await;
        n_logs = 10;

        router.wait_for_log(&btreeset![0], n_logs, timeout(), "send log to trigger snapshot").await?;
    }
    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(5000))
}
