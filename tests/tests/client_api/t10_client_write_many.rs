use std::sync::Arc;

use anyhow::Result;
use futures::prelude::*;
use maplit::btreeset;
use openraft::Config;
use openraft::raft::WriteResponse;
use openraft_memstore::ClientRequest;
use openraft_memstore::IntoMemClientRequest;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::log_id;
use crate::fixtures::ut_harness;

/// Test Raft::client_write_many() for batch writes
///
/// Test that multiple writes can be submitted in a single batch and results are returned in order.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn client_write_many() -> Result<()> {
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

    // Create multiple requests to write in batch
    let requests: Vec<ClientRequest> = (0..5).map(|i| ClientRequest::make_request("batch", i)).collect();

    // Submit batch and collect results from stream
    let mut stream = n0.client_write_many(requests).await?;

    let mut results: Vec<WriteResponse<TypeConfig>> = Vec::new();
    while let Some(result) = stream.try_next().await? {
        results.push(result?);
    }

    // Verify we got 5 results
    assert_eq!(5, results.len());

    // Verify log_ids are sequential
    for (i, resp) in results.iter().enumerate() {
        assert_eq!(log_id(1, 0, log_index + 1 + i as u64), resp.log_id);
    }

    log_index += 5;

    // Verify responses show previous values (state machine returns previous value for client_id)
    // First write has no previous value
    assert_eq!(None, results[0].response.0.as_deref());
    // Subsequent writes see previous serial numbers
    assert_eq!(Some("request-0"), results[1].response.0.as_deref());
    assert_eq!(Some("request-1"), results[2].response.0.as_deref());
    assert_eq!(Some("request-2"), results[3].response.0.as_deref());
    assert_eq!(Some("request-3"), results[4].response.0.as_deref());

    // Verify all nodes have applied the writes
    for id in [0, 1, 2] {
        router.wait(&id, None).applied_index(Some(log_index), "batch writes applied").await?;
    }

    Ok(())
}
