use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::RPCTypes;

use crate::fixtures::RaftRouter;
use crate::fixtures::rpc_request::RpcRequest;
use crate::fixtures::ut_harness;

/// The number of client writes issued one after another, each awaited before the next.
const WRITES: usize = 100;

/// A commit that an AppendEntries already carried must not trigger a second, entry-less
/// AppendEntries.
///
/// Issue 2004: `next_request()` used to read the commit watch without marking it seen, so the
/// pending `committed_rx.changed()` woke the replication stream once more and produced an extra
/// AppendEntries with no entries and the same `leader_commit`.
///
/// Every commit still needs to reach the follower, so one entry-less AppendEntries per
/// log-carrying one remains expected: a sequential writer leaves no later write to piggyback the
/// commit on.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn no_redundant_commit_only_append_entries() -> Result<()> {
    let config = Arc::new(
        Config {
            // Heartbeats send entry-less AppendEntries of their own, which would blur the count.
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    let mut log_index = router.new_cluster(btreeset! {0, 1, 2}, btreeset! {}).await?;

    let entry_less = Arc::new(AtomicU64::new(0));
    let with_entries = Arc::new(AtomicU64::new(0));

    tracing::info!(log_index, "--- count AppendEntries sent from now on");
    {
        let entry_less = entry_less.clone();
        let with_entries = with_entries.clone();

        router
            .set_rpc_pre_hook(RPCTypes::AppendEntries, move |_router, req, _from, _to| {
                if let RpcRequest::AppendEntries(append) = &req {
                    if append.entries.is_empty() {
                        entry_less.fetch_add(1, Ordering::Relaxed);
                    } else {
                        with_entries.fetch_add(1, Ordering::Relaxed);
                    }
                }
                Box::pin(async move { Ok(()) })
            })
            .await;
    }

    tracing::info!(log_index, "--- write {} logs, one at a time", WRITES);
    {
        log_index += router.client_request_many(0, "foo", WRITES).await?;

        router.wait(&1, timeout()).applied_index(Some(log_index), "node 1 applied").await?;
        router.wait(&2, timeout()).applied_index(Some(log_index), "node 2 applied").await?;
    }

    let entry_less = entry_less.load(Ordering::Relaxed);
    let with_entries = with_entries.load(Ordering::Relaxed);

    tracing::info!(entry_less, with_entries, "--- AppendEntries counts");

    assert!(
        entry_less <= with_entries,
        "expect at most one entry-less AppendEntries per log-carrying one, \
         but got {} entry-less and {} with entries",
        entry_less,
        with_entries
    );

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(5_000))
}
