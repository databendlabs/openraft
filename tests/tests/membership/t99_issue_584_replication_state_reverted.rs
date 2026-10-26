use anyhow::Result;
use openraft::Config;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn t99_issue_584_replication_state_reverted() -> Result<()> {
    // - Add a learner and replicate all logs to it.
    // - Add the learner as a voter. When membership changes, openraft internally restarts all
    //   replication.
    //
    // This case asserts it does not break the internal monotonic-replication-progress guarantee.

    let config = Config {
        max_in_snapshot_log_to_keep: 2000, // prevent snapshot
        enable_tick: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    let mut log_index = router.new_cluster([0], [1]).await?;

    let n = 500u64;
    tracing::info!(log_index, "--- write up to {} logs", n);
    {
        router.client_request_many(0, "foo", (n - log_index) as usize).await?;
        log_index = n;

        router.wait(&1).applied_index(Some(log_index), "replicate all logs to learner").await?;
    }

    tracing::info!(
        log_index,
        "--- change-membership: make learner node-1 a voter. This should not panic"
    );
    {
        let leader = router.get_raft_handle(&0)?;
        leader.change_membership([0, 1], false).await?;
        log_index += 1; // 1 change_membership log

        let _ = log_index;
    }

    Ok(())
}
