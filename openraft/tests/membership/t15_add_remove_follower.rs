use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::ServerState;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// When a node is removed from cluster, replication to it should be stopped.
///
/// - brings 5 nodes online: one leader and 4 follower.
/// - asserts that the leader was able to successfully commit logs and that the followers has successfully replicated
///   the payload.
/// - remove one follower: node-4
/// - asserts node-4 becomes learner and the leader stops sending logs to it.
#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn add_remove_voter() -> Result<()> {
    let c01234 = btreeset![0, 1, 2, 3, 4];
    let c0123 = btreeset![0, 1, 2, 3];

    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let mut log_index = router.new_nodes_from_single(c01234.clone(), btreeset! {}).await?;

    tracing::info!("--- write 100 logs");
    {
        router.client_request_many(0, "client", 100).await?;
        log_index += 100;

        router.wait_for_log(&c01234, Some(log_index), timeout(), "write 100 logs").await?;
    }

    tracing::info!("--- remove n{}", 4);
    {
        let node = router.get_raft_handle(&0)?;
        node.change_membership(c0123.clone(), true, false).await?;
        log_index += 2; // two member-change logs

        router.wait_for_log(&c0123, Some(log_index), timeout(), "removed node-4 from membership").await?;
    }

    tracing::info!("--- write another 100 logs");
    {
        router.client_request_many(0, "client", 100).await?;
        log_index += 100;
    }

    router.wait_for_log(&c0123, Some(log_index), timeout(), "4 nodes recv logs 100~200").await?;

    tracing::info!("--- log will not be sync to removed node");
    {
        let x = router.latest_metrics();
        assert!(x[4].last_log_index < Some(log_index - 50));
    }

    router
        .wait(&4, timeout())
        .metrics(
            |x| x.state == ServerState::Learner || x.state == ServerState::Candidate,
            "node-4 is left a learner or follower, depending on if it received the uniform config",
        )
        .await?;

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2000))
}
