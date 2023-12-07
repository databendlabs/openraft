use std::sync::Arc;
use std::time::Duration;

use anyerror::AnyError;
use anyhow::Result;
use maplit::btreeset;
use openraft::error::RPCError;
use openraft::error::Unreachable;
use openraft::Config;
use openraft::RPCTypes;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Append-entries should backoff when a `Unreachable` error is found.
#[async_entry::test(worker_threads = 4, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn append_entries_backoff() -> Result<()> {
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

    tracing::info!("--- initializing cluster");
    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    let counts0 = router.get_rpc_count();
    let n = 10u64;

    tracing::info!(log_index, "--- set node 2 to unreachable, and write 10 entries");
    {
        router.set_rpc_pre_hook(RPCTypes::AppendEntries, |_router, _req, _id, target| {
            if target == 2 {
                let any_err = AnyError::error("unreachable");
                Err(RPCError::Unreachable(Unreachable::new(&any_err)))
            } else {
                Ok(())
            }
        });
        // The above is equivalent to the following:
        // router.set_unreachable(2, true);

        router.client_request_many(0, "0", n as usize).await?;
        log_index += n;

        router.wait(&0, timeout()).applied_index(Some(log_index), format!("{} writes", n)).await?;
    }

    let counts1 = router.get_rpc_count();

    let c0 = *counts0.get(&RPCTypes::AppendEntries).unwrap_or(&0);
    let c1 = *counts1.get(&RPCTypes::AppendEntries).unwrap_or(&0);

    // Without backoff, the leader would send about 40 append-entries RPC.
    // 20 for append log entries, 20 for updating committed.
    assert!(
        n < c1 - c0 && c1 - c0 < n * 4,
        "append-entries should backoff when a `Unreachable` error is found"
    );

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1_000))
}
