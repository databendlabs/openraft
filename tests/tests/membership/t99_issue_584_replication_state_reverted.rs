use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn t99_issue_584_replication_state_reverted() -> Result<()> {
    // - Add a learner and replicate all logs to it.
    // - Add the learner as a voter. When membership changes, openraft internally restarts all
    //   replication.
    //
    // This case asserts it does not break the internal monotonic-replication-progress guarantee.

    let config = Arc::new(
        Config {
            max_in_snapshot_log_to_keep: 2000, // prevent snapshot
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let mut log_index = router.new_cluster(btreeset! {0}, btreeset! {1}).await?;

    let n = 500u64;
    tracing::info!(log_index, "--- write up to {} logs", n);
    {
        router.client_request_many(0, "foo", (n - log_index) as usize).await?;
        log_index = n;

        router.wait(&1, timeout()).applied_index(Some(log_index), "replicate all logs to learner").await?;
    }

    tracing::info!(
        log_index,
        "--- change-membership: make learner node-1 a voter. This should not panic"
    );
    {
        let leader = router.get_raft_handle(&0)?;
        leader.change_membership([0, 1], false).await?;
        log_index += 2; // 2 change_membership log

        let _ = log_index;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
