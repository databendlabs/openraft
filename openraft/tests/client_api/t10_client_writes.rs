use std::sync::Arc;

use anyhow::Result;
use futures::prelude::*;
use maplit::btreeset;
use openraft::Config;
use openraft::LeaderId;
use openraft::LogId;
use openraft::SnapshotPolicy;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::RaftRouter;

/// Client write tests.
///
/// What does this test do?
///
/// - create a stable 3-node cluster.
/// - write a lot of data to it.
/// - assert that the cluster stayed stable and has all of the expected data.
#[async_entry::test(worker_threads = 4, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn client_writes() -> Result<()> {
    let config = Arc::new(
        Config {
            snapshot_policy: SnapshotPolicy::LogsSinceLast(500),
            // The write load is heavy in this test, need a relatively long timeout.
            election_timeout_min: 500,
            election_timeout_max: 1000,
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let mut log_index = router.new_nodes_from_single(btreeset! {0,1,2}, btreeset! {}).await?;

    // Write a bunch of data and assert that the cluster stayes stable.
    let leader = router.leader().expect("leader not found");
    let mut clients = futures::stream::FuturesUnordered::new();
    clients.push(router.client_request_many(leader, "0", 100));
    clients.push(router.client_request_many(leader, "1", 100));
    clients.push(router.client_request_many(leader, "2", 100));
    clients.push(router.client_request_many(leader, "3", 100));
    clients.push(router.client_request_many(leader, "4", 100));
    clients.push(router.client_request_many(leader, "5", 100));
    while clients.next().await.is_some() {}

    log_index += 100 * 6;
    router.wait_for_log(&btreeset![0, 1, 2], Some(log_index), None, "sync logs").await?;

    router.assert_stable_cluster(Some(1), Some(log_index)); // The extra 1 is from the leader's initial commit entry.

    router
        .assert_storage_state(
            1,
            log_index,
            Some(0),
            LogId::new(LeaderId::new(1, 0), log_index),
            Some(((499..600).into(), 1)),
        )
        .await?;

    Ok(())
}
