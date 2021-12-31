use std::sync::Arc;

use anyhow::Result;
use async_raft::Config;
use async_raft::State;
use fixtures::RaftRouter;
use maplit::btreeset;

#[macro_use]
mod fixtures;

/// Lagging network test.
///
/// What does this test do?
///
/// - Setup a network with <=50 ms random delay of messages.
/// - bring a single-node cluster online.
/// - add two NonVoter and then try to commit one log.
/// - change config to a 3 members cluster and commit another log.
///
/// RUST_LOG=async_raft,memstore,lagging_network_write=trace cargo test -p async-raft --test lagging_network_write
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn lagging_network_write() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    let timeout = Some(tokio::time::Duration::from_millis(2000));

    let config = Arc::new(
        Config {
            heartbeat_interval: 100,
            election_timeout_min: 300,
            election_timeout_max: 600,
            ..Default::default()
        }
        .validate()?,
    );
    let router = RaftRouter::builder(config).send_delay(50).build();
    let router = Arc::new(router);

    router.new_raft_node(0).await;
    let mut want = 0;

    // Assert all nodes are in non-voter state & have no entries.
    router.wait_for_log(&btreeset![0], want, timeout, "empty").await?;
    router.wait_for_state(&btreeset![0], State::NonVoter, None, "empty").await?;
    router.assert_pristine_cluster().await;

    // Initialize the cluster, then assert that a stable cluster was formed & held.
    tracing::info!("--- initializing cluster");
    router.initialize_from_single_node(0).await?;
    want += 1;

    router.wait_for_log(&btreeset![0], want, timeout, "init").await?;
    router.wait_for_state(&btreeset![0], State::Leader, None, "init").await?;
    router.assert_stable_cluster(Some(1), Some(want)).await;

    // Sync some new nodes.
    router.new_raft_node(1).await;
    router.add_learner(0, 1).await?;

    router.new_raft_node(2).await;
    router.add_learner(0, 2).await?;

    router.wait_for_log(&btreeset![1, 2], want, timeout, "non-voter init").await?;

    router.client_request_many(0, "client", 1).await;
    want += 1;
    router.wait_for_log(&btreeset![0, 1, 2], want, timeout, "write one log").await?;

    router.change_membership(0, btreeset![0, 1, 2]).await?;
    want += 2;
    router.wait_for_state(&btreeset![0], State::Leader, None, "changed").await?;
    router.wait_for_state(&btreeset![1, 2], State::Follower, None, "changed").await?;
    router.wait_for_log(&btreeset![0, 1, 2], want, timeout, "3 candidates").await?;

    router.client_request_many(0, "client", 1).await;
    want += 1;
    router.wait_for_log(&btreeset![0, 1, 2], want, timeout, "write 2nd log").await?;

    Ok(())
}
