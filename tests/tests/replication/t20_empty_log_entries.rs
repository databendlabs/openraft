use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::ClientRequest;
use openraft_memstore::IntoMemClientRequest;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// Test that replication continues gracefully when `limited_get_log_entries` returns empty.
///
/// This tests the fix for issue #1601: instead of panicking, the replication should
/// treat empty results as a heartbeat and continue.
///
/// Uses a 3-node cluster so the leader can still commit with 2/3 quorum even when
/// replication to one node is degraded.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn empty_limited_get_log_entries() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing 3-node cluster");
    let mut log_index = router.new_cluster(btreeset! {0, 1, 2}, btreeset! {}).await?;

    tracing::info!(log_index, "--- write some logs to establish replication");
    log_index += router.client_request_many(0, "foo", 5).await?;
    router.wait(&1, timeout()).applied_index(Some(log_index), "node 1 replicated").await?;
    router.wait(&2, timeout()).applied_index(Some(log_index), "node 2 replicated").await?;

    tracing::info!(log_index, "--- isolate node 2 so leader replicates only to node 1");

    tracing::info!(
        log_index,
        "--- set leader's storage to return empty from limited_get_log_entries"
    );
    router.set_return_empty_limited_get(&0, true)?;

    tracing::info!(
        log_index,
        "--- write more logs; replication should continue without panic"
    );
    {
        // The leader will try to replicate the log.
        // When limited_get_log_entries returns empty, it should not panic
        // but instead treat it as a heartbeat.
        //
        // Node 1 won't receive the actual log because the leader's storage returns empty,
        // so commit won't happen. Just sleep a bit and verify no panic occurred.

        // Send a fire-and-forget write to the leader - this won't wait for commit
        let raft = router.get_raft_handle(&0)?;
        raft.client_write_ff(ClientRequest::make_request("bar", 1), None).await?;

        // Give replication some time to attempt sending - this is where it would panic
        // if the empty handling wasn't working correctly.
        TypeConfig::sleep(Duration::from_millis(800)).await;

        // Access raft core, expect no error, because there should not be a panic
        raft.with_raft_state(|_| ()).await?;

        // If we get here without panic, the fix is working.
        tracing::info!(log_index, "--- no panic occurred, fix is working");
    }

    // Clear the flag to allow clean shutdown
    router.set_return_empty_limited_get(&0, false)?;
    log_index += 1;

    router.wait(&1, timeout()).applied_index(Some(log_index), "node 1 replicated").await?;
    router.wait(&2, timeout()).applied_index(Some(log_index), "node 2 replicated").await?;
    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2_000))
}
