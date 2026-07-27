use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// One RPC round trip. It is also the AppendEntries timeout, i.e. the longest a replication task
/// can stay inside a call to an unresponsive follower.
const RPC_TIMEOUT: u64 = 5_000;

/// Closing a replication task that is inside a follower RPC must not block RaftCore.
///
/// Rebuilding the replication streams removes the previous tasks. Joining them inline made
/// RaftCore wait until every removed task finished its current RPC, so one unresponsive follower
/// froze the leader for a full RPC timeout per removed target, blocking membership changes, writes
/// and metrics alike.
///
/// Node 0 is the only voter, so a quorum never depends on the unresponsive node, and node 1 is a
/// learner, which never campaigns. The only thing that can delay the membership change below is
/// the defect under test.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn close_replication_inside_follower_rpc() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            heartbeat_interval: RPC_TIMEOUT,
            election_timeout_min: RPC_TIMEOUT * 2,
            election_timeout_max: RPC_TIMEOUT * 2 + 1_000,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster: voter 0, learner 1");
    let mut log_index = router.new_cluster(btreeset! {0}, btreeset! {1}).await?;

    tracing::info!(log_index, "--- node 1 stops responding to AppendEntries");
    router.set_rpc_blocked(1, true);

    tracing::info!(
        log_index,
        "--- a write commits on the single voter, replication to node 1 stalls"
    );
    log_index += router.client_request_many(0, "foo", 1).await?;
    // Let the replication task enter the RPC it will not return from for `RPC_TIMEOUT`.
    tokio::time::sleep(Duration::from_millis(300)).await;

    tracing::info!(log_index, "--- adding a learner rebuilds the replication streams");
    router.new_raft_node(2).await;
    {
        let raft = router.get_raft_handle(&0)?;
        let res = tokio::time::timeout(Duration::from_millis(1_500), raft.add_learner(2, (), false)).await;
        assert!(
            res.is_ok(),
            "RaftCore must not wait for the replication task that sits in node 1's RPC"
        );
        res.unwrap()?;
    }
    log_index += 1;

    tracing::info!(
        log_index,
        "--- RaftCore keeps serving writes and the new learner replicates"
    );
    log_index += router.client_request_many(0, "bar", 1).await?;
    router.wait(&2, timeout()).applied_index(Some(log_index), "new learner replicated").await?;

    tracing::info!(log_index, "--- node 1 responds again and catches up");
    router.set_rpc_blocked(1, false);
    log_index += router.client_request_many(0, "wow", 1).await?;
    router.wait(&1, timeout()).applied_index(Some(log_index), "node 1 recovered").await?;

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(3_000))
}
