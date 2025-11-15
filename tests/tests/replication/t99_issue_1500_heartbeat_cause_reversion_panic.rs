use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use openraft::Config;
use openraft::RPCTypes;

use crate::fixtures::RaftRouter;
use crate::fixtures::rpc_request::RpcRequest;
use crate::fixtures::ut_harness;

/// Test heartbeat does not cause false log reversion panic when follower lags behind.
///
/// Issue: A lagging follower receiving a heartbeat with `prev_log_id` set to the committed
/// log id causes a conflict response. When this delayed conflict arrives after the follower
/// catches up, the leader incorrectly interprets it as log reversion and panics.
///
/// Before fix: Leader used `prev_log_id = committed` in heartbeats, which may not exist on
/// the follower yet, causing false conflicts.
///
/// After fix: Leader uses `prev_log_id = matching` (last confirmed replicated log id),
/// which is guaranteed to exist on the follower.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn t99_issue_1500_heartbeat_cause_reversion_panic() -> anyhow::Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: true,
            allow_log_reversion: Some(false),
            election_timeout_min: 800,
            election_timeout_max: 801,
            heartbeat_interval: 100,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- bring up cluster of 3 node");
    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    // Set a hook to delay heartbeat response from node 2, but bypass normal replication
    // (non-empty payload). This creates a race condition where the heartbeat conflict response
    // may arrive after the follower has caught up via normal replication.
    router
        .set_rpc_post_hook(RPCTypes::AppendEntries, |_router, req, _resp, _id, target| {
            let sleep_ms = if target == 2 {
                tracing::debug!("Post-hook for target {}: {}", target, req);
                match req {
                    RpcRequest::AppendEntries(append) => {
                        if append.entries.is_empty() {
                            10
                        } else {
                            0
                        }
                    }
                    _ => 0,
                }
            } else {
                0
            };

            let fu = async move {
                if sleep_ms > 0 {
                    tracing::debug!("Post-hook for target {}: delaying response by {}ms", target, sleep_ms);
                    tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
                    tracing::debug!("Post-hook for target {}: delay complete", target);
                }
                Ok::<_, _>(())
            };

            Box::pin(fu)
        })
        .await;

    tracing::info!(log_index, "--- write some logs");
    {
        log_index += router.client_request_many(0, "foo", 500).await?;
        router.wait(&0, timeout()).applied_index(Some(log_index), "commit all written entries").await?;
    }

    tracing::info!(log_index, "--- check if panic occurs");
    {
        let n0 = router.get_raft_handle(&0)?;
        n0.with_raft_state(|_st| ()).await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
