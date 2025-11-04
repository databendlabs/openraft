use std::sync::Arc;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::impls::ProgressResponder;
use openraft::raft::ClientWriteResult;
use openraft_memstore::ClientRequest;
use openraft_memstore::IntoMemClientRequest;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// Test Raft::client_write_ff with ProgressResponder
///
/// Verify that on_commit() is called when a log entry is committed,
/// and on_complete() is called when the entry is applied.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn client_write_ff_with_progress_responder() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    let n0 = router.get_raft_handle(&0)?;

    // Test with ProgressResponder - first write
    let (responder, commit_rx, complete_rx) = ProgressResponder::new();
    n0.client_write_ff(ClientRequest::make_request("foo", 10), Some(responder)).await?;

    log_index += 1;

    // Wait for commit notification
    let commit_log_id = commit_rx.await?;
    tracing::info!("Received commit notification for log_id: {:?}", commit_log_id);
    assert_eq!(log_id(1, 0, log_index), commit_log_id);

    // Wait for completion
    let result: ClientWriteResult<TypeConfig> = complete_rx.await?;
    tracing::info!("Received completion response: {:?}", result);
    let response = result?;
    assert_eq!(log_id(1, 0, log_index), response.log_id);
    // First write returns None (no previous value)
    assert_eq!(None, response.response().0.as_deref());

    // Test another write - pattern follows t10: write("foo", 11) with no responder, then write("foo",
    // 12)
    n0.client_write_ff(ClientRequest::make_request("foo", 11), None).await?;
    log_index += 1;

    let (responder, commit_rx, complete_rx) = ProgressResponder::new();
    n0.client_write_ff(ClientRequest::make_request("foo", 12), Some(responder)).await?;
    log_index += 1;

    // Wait for commit notification on second write
    let commit_log_id_2 = commit_rx.await?;
    tracing::info!("Received second commit notification for log_id: {:?}", commit_log_id_2);
    assert_eq!(log_id(1, 0, log_index), commit_log_id_2);

    let result: ClientWriteResult<TypeConfig> = complete_rx.await?;
    let response = result?;
    assert_eq!(log_id(1, 0, log_index), response.log_id);
    // Should return the value from the previous write (11)
    assert_eq!(Some("request-11"), response.response().0.as_deref());

    Ok(())
}
