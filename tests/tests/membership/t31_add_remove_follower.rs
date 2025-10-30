use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::ServerState;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// When a node is removed from cluster, replication to it should be stopped.
///
/// - brings 5 nodes online: one leader and 4 follower.
/// - asserts that the leader was able to successfully commit logs and that the followers has
///   successfully replicated the payload.
/// - remove one follower: node-4
/// - asserts node-4 becomes learner and the leader stops sending logs to it.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
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

    let mut log_index = router.new_cluster(c01234.clone(), btreeset! {}).await?;

    tracing::info!(log_index, "--- write 100 logs");
    {
        router.client_request_many(0, "client", 100).await?;
        log_index += 100;

        for id in c01234.iter() {
            router.wait(id, timeout()).applied_index(Some(log_index), "write 100 logs").await?;
        }
    }

    tracing::info!(log_index, "--- remove n{}", 4);
    {
        let node = router.get_raft_handle(&0)?;
        node.change_membership(c0123.clone(), false).await?;
        log_index += 2; // two member-change logs

        for id in c0123.iter() {
            router.wait(id, timeout()).applied_index(Some(log_index), "removed node-4 from membership").await?;
        }
    }

    tracing::info!(log_index, "--- write another 100 logs");
    {
        router.client_request_many(0, "client", 100).await?;
        log_index += 100;
    }

    for id in c0123.iter() {
        router.wait(id, timeout()).applied_index(Some(log_index), "4 nodes recv logs 100~200").await?;
    }

    tracing::info!(log_index, "--- log will not be sync to removed node");
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
