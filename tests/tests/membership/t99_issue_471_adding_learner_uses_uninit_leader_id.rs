use anyhow::Result;
use openraft::Config;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// When adding learner and waiting for the learner to become up to date,
/// it should not try to use `matched.leader_id` which may be uninitialized, i.e., `(0,0)`.
/// https://github.com/databendlabs/openraft/issues/471
///
/// - Brings up 1 leader.
/// - Add learner at once.
/// - It should not panic.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn adding_learner_do_not_use_matched_leader_id() -> Result<()> {
    let config = Config {
        // Replicate log one by one, to trigger a state report with matched=(0,0,0), which is
        // the first log id.
        max_payload_entries: 1,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    router.new_cluster([0], []).await?;

    tracing::info!("--- feed 2 log to make replication busy");
    {
        router.client_request_many(0, "foo", 2).await?;
    }

    // Delay replication.
    router.network_send_delay(100);

    tracing::info!("--- add learner: node-1");
    {
        router.new_raft_node(1).await?;
        router.add_learner(0, 1).await?;
    }

    Ok(())
}
