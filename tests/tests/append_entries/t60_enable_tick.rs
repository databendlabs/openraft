use std::time::Duration;

use anyhow::Result;
use openraft::Config;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::expect_timeout;
use crate::fixtures::timeout;
use crate::fixtures::ut_harness;

/// Re-enabling the heartbeat timer with `tick(true)` resumes the heartbeats of an idle leader.
///
/// With the timer disabled from the start, nothing wakes the idle leader. Turning it on wakes the
/// leader, which sends the overdue heartbeat at once, and the timer then keeps firing.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn enable_tick_resumes_heartbeats() -> Result<()> {
    let config = Config {
        enable_tick: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- create cluster of 0,1,2 with the heartbeat timer disabled; node 0 becomes leader");
    let log_index = router.new_cluster([0, 1, 2], []).await?;
    let n0 = router.get_raft_handle(&0)?;

    tracing::info!(
        log_index,
        "--- with the timer disabled the leader sends nothing, so no later RPC is acknowledged"
    );
    {
        let now = TypeConfig::now();
        let got = n0
            .wait(Some(Duration::from_millis(500)))
            .leader_with_quorum_acked(Some(now), "an RPC sent after now acknowledged")
            .await;
        expect_timeout(got)?;
    }

    tracing::info!(
        log_index,
        "--- enable the timer; every round of heartbeats is acknowledged by a quorum"
    );
    {
        n0.runtime_config().tick(true);

        for round in 0..3 {
            let now = TypeConfig::now();
            n0.wait(timeout())
                .leader_with_quorum_acked(Some(now), format!("heartbeat round {round} acknowledged"))
                .await?;
        }
    }

    Ok(())
}
