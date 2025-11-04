use std::sync::Arc;

use anyhow::Result;
use futures::prelude::*;
use maplit::btreeset;
use openraft::Config;
use openraft::SnapshotPolicy;
use openraft::impls::ProgressResponder;
use openraft::raft::ClientWriteResponse;
use openraft_memstore::ClientRequest;
use openraft_memstore::IntoMemClientRequest;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// - create a stable 3-node cluster.
/// - write a lot of data to it.
/// - assert that the cluster stayed stable and has all of the expected data.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
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
    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

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
    for id in [0, 1, 2] {
        router.wait(&id, None).applied_index(Some(log_index), "sync logs").await?;
    }

    router
        .assert_storage_state(
            1,
            log_index,
            Some(0),
            log_id(1, 0, log_index),
            Some(((499..600).into(), 1)),
        )
        .await?;

    Ok(())
}

/// Test Raft::client_write_ff,
///
/// Manually receive the client-write response via the returned `Responder::Receiver`
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn client_write_ff() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;
    let _ = log_index;

    let n0 = router.get_raft_handle(&0)?;

    let (responder, _commit_rx, complete_rx) = ProgressResponder::new();
    n0.client_write_ff(ClientRequest::make_request("foo", 2), Some(responder)).await?;
    let got: ClientWriteResponse<TypeConfig> = complete_rx.await??;
    assert_eq!(None, got.response().0.as_deref());

    // Deliberately set the responder to None and do not wait for the result.
    n0.client_write_ff(ClientRequest::make_request("foo", 3), None).await?;

    let (responder, _commit_rx, complete_rx) = ProgressResponder::new();
    n0.client_write_ff(ClientRequest::make_request("foo", 4), Some(responder)).await?;
    let got: ClientWriteResponse<TypeConfig> = complete_rx.await??;
    assert_eq!(Some("request-3"), got.response().0.as_deref());

    Ok(())
}
