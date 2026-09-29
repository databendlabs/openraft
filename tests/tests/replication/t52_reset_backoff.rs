use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::RPCTypes;
use openraft::async_runtime::WatchReceiver;
use openraft::errors::RPCError;
use openraft::errors::Unreachable;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// A constant 5-second backoff delay, much longer than `timeout()`.
const BACKOFF: &str = "5s ...5s";

/// `Trigger::reset_backoff()` ends the active backoff wait, so a reachable follower catches up
/// before the backoff delay ends.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn reset_backoff_ends_active_backoff() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_tick: false,
            backoff: BACKOFF.to_string(),
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- bring up a 3-node cluster");
    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    let n0 = router.get_raft_handle(&0)?;
    let n2 = router.get_raft_handle(&2)?;

    let failures_remaining = Arc::new(AtomicU64::new(0));
    {
        let failures_remaining = failures_remaining.clone();
        router
            .set_rpc_pre_hook(RPCTypes::AppendEntries, move |_router, _req, _from, to| {
                let should_fail = to == 2 && failures_remaining.load(Ordering::SeqCst) > 0;
                let res = if should_fail {
                    failures_remaining.fetch_sub(1, Ordering::SeqCst);
                    Err(RPCError::Unreachable(Unreachable::<TypeConfig>::from_string(
                        "injected",
                    )))
                } else {
                    Ok(())
                };
                Box::pin(futures::future::ready(res))
            })
            .await;
    }

    tracing::info!(log_index, "--- fail one AppendEntries to node-2 to start its backoff");
    {
        failures_remaining.store(1, Ordering::SeqCst);

        router.client_request(0, "foo", 1).await?;
        log_index += 1;
    }

    tracing::info!(log_index, "--- node-2 is reachable but the backoff keeps it behind");
    {
        TypeConfig::sleep(Duration::from_millis(500)).await;

        let failures_remaining = failures_remaining.load(Ordering::SeqCst);
        assert_eq!(failures_remaining, 0, "the injected failure is consumed");

        let n2_metrics = n2.metrics();
        let last_log_index = n2_metrics.borrow_watched().last_log_index;
        assert_eq!(
            last_log_index,
            Some(log_index - 1),
            "node-2 has not received the new log"
        );
    }

    tracing::info!(log_index, "--- reset the backoff, node-2 catches up within timeout()");
    {
        n0.trigger().reset_backoff([2]).await?;

        router.wait(&2, timeout()).applied_index(Some(log_index), "node-2 catches up after reset").await?;
    }

    Ok(())
}

/// A reset sent while no backoff is active is dropped when the next backoff starts, so it does
/// not skip the first wait of that backoff.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn reset_backoff_before_backoff_is_dropped() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_tick: false,
            backoff: BACKOFF.to_string(),
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- bring up a 3-node cluster");
    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    let n0 = router.get_raft_handle(&0)?;

    tracing::info!(log_index, "--- reset the backoff to node-2 while its stream is healthy");
    {
        n0.trigger().reset_backoff([2]).await?;
    }

    let attempts = Arc::new(AtomicU64::new(0));

    tracing::info!(log_index, "--- cut off node-2 and write a log to start its backoff");
    {
        let attempts = attempts.clone();
        router
            .set_rpc_pre_hook(RPCTypes::AppendEntries, move |_router, _req, _from, to| {
                let res = if to == 2 {
                    attempts.fetch_add(1, Ordering::SeqCst);
                    Err(RPCError::Unreachable(Unreachable::<TypeConfig>::from_string(
                        "injected",
                    )))
                } else {
                    Ok(())
                };
                Box::pin(futures::future::ready(res))
            })
            .await;

        router.client_request(0, "foo", 1).await?;
        log_index += 1;
    }

    tracing::info!(log_index, "--- the earlier reset must not skip the first backoff wait");
    {
        TypeConfig::sleep(Duration::from_millis(1_000)).await;

        let attempts = attempts.load(Ordering::SeqCst);
        assert_eq!(attempts, 1, "node-2 is retried only after the backoff delay");
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
