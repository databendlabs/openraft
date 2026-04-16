use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::RPCTypes;
use openraft::errors::RPCError;
use openraft::errors::Unreachable;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// Regression test for [issue #1723](https://github.com/databendlabs/openraft/issues/1723):
/// backoff must be cleared on the first successful RPC inside a stream replication
/// session, not left for the outer main loop to clear — the outer loop does not run
/// while a session is alive.
///
/// Reproducing the bug in a test is subtle because the 0.10 engine, after any error,
/// issues a short "commit-update" bounded session (`LogIdRange` with an empty range)
/// *before* the long pipeline session resumes. That single-RPC bounded session
/// absorbs the one backoff sleep, then the main loop runs and clears the backoff
/// because `rank` was reset to 0 on that success. By the time the long pipeline
/// session starts, the backoff is already gone.
///
/// This test defeats that masking by injecting **two** transient errors:
///   - Error 1 ends the active pipeline session; `rank = 100`; main loop enables backoff.
///   - Error 2 kills the bounded commit-update session; `rank = 200`; main loop keeps backoff
///     enabled.
///   - The next session is a long-lived pipeline (`LogsSince`). It starts with `rank = 200` and
///     `Backoff = Some(...)`. Its first RPC succeeds and resets `rank = 0`, but the bug leaves
///     `Backoff` in place. Every subsequent RPC (one per client write, since quorum = 2 in this
///     2-node cluster) pays the ~200ms sleep until the session ends.
///
/// Writing `n = 10` entries serially therefore takes roughly `n * 200ms = 2000ms`
/// with the bug versus tens of ms without.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn backoff_cleared_after_success_in_stream() -> Result<()> {
    let config = Arc::new(
        Config {
            heartbeat_interval: 5_000,
            election_timeout_min: 10_000,
            election_timeout_max: 10_001,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing 2-node cluster");
    router.new_cluster(btreeset! {0, 1}, btreeset! {}).await?;

    // Inject two transient `Unreachable` errors on AppendEntries to node 1. See the
    // doc comment above for why two are needed.
    let failures_remaining = Arc::new(AtomicU64::new(2));
    {
        let failures_remaining = failures_remaining.clone();
        router
            .set_rpc_pre_hook(RPCTypes::AppendEntries, move |_router, _req, _id, target| {
                let should_fail = target == 1 && failures_remaining.load(Ordering::SeqCst) > 0;
                let res = if should_fail {
                    failures_remaining.fetch_sub(1, Ordering::SeqCst);
                    Err(RPCError::Unreachable(Unreachable::<TypeConfig>::from_string(
                        "transient",
                    )))
                } else {
                    Ok(())
                };
                Box::pin(futures::future::ready(res))
            })
            .await;
    }

    tracing::info!("--- write one entry to trigger both injected errors");
    router.client_request_many(0, "trigger", 1).await?;

    // Give the replication task time to consume both errors and enter the long
    // pipeline session with backoff enabled.
    TypeConfig::sleep(Duration::from_millis(300)).await;

    assert_eq!(
        failures_remaining.load(Ordering::SeqCst),
        0,
        "precondition: both injected errors must have been consumed"
    );

    let n: u64 = 10;
    tracing::info!("--- write {} entries and time commit latency", n);
    let start = TypeConfig::now();
    router.client_request_many(0, "after", n as usize).await?;
    let elapsed = TypeConfig::now() - start;

    tracing::info!("--- {} post-recovery writes committed in {:?}", n, elapsed);

    // With the bug: each commit waits ~200ms for the throttled follower, so n serial
    // writes take on the order of n * 200ms = 2000ms (plus commit-update RPCs).
    // Without the bug: writes complete in tens of ms.
    assert!(
        elapsed < Duration::from_millis(1_000),
        "{} post-recovery writes took {:?}; backoff is stuck after success \
         (expected < 1000ms, buggy behavior is ~{}ms)",
        n,
        elapsed,
        n * 200,
    );

    Ok(())
}
