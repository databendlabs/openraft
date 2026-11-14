use anyhow::Result;
use futures::prelude::*;
use openraft::Config;
use openraft::errors::ClientWriteError;
use openraft::errors::ForwardReason;
use openraft::raft::WriteOption;
use openraft::raft::WriteResponse;
use openraft::type_config::alias::LeaderIdOf;
use openraft::vote::RaftLeaderId;
use openraft_memstore::ClientRequest;
use openraft_memstore::ClientResponse;
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
    let config = Config {
        enable_tick: false,
        ..Default::default()
    };

    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initializing cluster");
    let mut log_index = router.new_cluster([0, 1, 2], []).await?;

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
        router.wait(&id).applied_index(Some(log_index), "batch writes applied").await?;
    }

    Ok(())
}

/// `Raft::client_write_many_with_option()` appends a batch whose expected leader matches, and
/// yields `ForwardToLeader` for every entry of a batch whose expected leader does not.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn client_write_many_with_option() -> Result<()> {
    let config = Config {
        enable_tick: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initialize a 3-node cluster");
    let mut log_index = router.new_cluster([0, 1, 2], []).await?;
    let n0 = router.get_raft_handle(&0)?;

    let leader_id = n0.as_leader()?.to_committed_leader_id();
    let fake_leader = LeaderIdOf::<TypeConfig>::new(100, 99).to_committed();

    tracing::info!(
        log_index,
        "--- a batch expecting the current leader yields applied results"
    );
    {
        let requests = (1..=2).map(|i| ClientRequest::make_request("foo", i));
        let write_option = WriteOption::default().with_leader(leader_id);
        let results: Vec<_> = n0.client_write_many_with_option(requests, write_option).await?.try_collect().await?;

        assert_eq!(2, results.len());
        for (i, result) in results.into_iter().enumerate() {
            let response = result?;
            assert_eq!(log_id(1, 0, log_index + 1 + i as u64), response.log_id);
        }
        log_index += 2;
    }

    tracing::info!(
        log_index,
        "--- a batch expecting another leader yields ForwardToLeader for every entry"
    );
    {
        let requests = (3..=4).map(|i| ClientRequest::make_request("foo", i));
        let write_option = WriteOption::default().with_leader(fake_leader);
        let results: Vec<_> = n0.client_write_many_with_option(requests, write_option).await?.try_collect().await?;

        assert_eq!(2, results.len());
        for result in results {
            let err = result.unwrap_err();
            let ClientWriteError::ForwardToLeader(forward) = err else {
                panic!("expected ForwardToLeader, got: {:?}", err)
            };
            assert_eq!(Some(0), forward.leader_id);
            assert_eq!(ForwardReason::NotLeader, forward.reason);
        }
    }

    tracing::info!(
        log_index,
        "--- the next write follows request-2, so the rejected batch was not applied"
    );
    {
        let mut results = n0
            .client_write_many_with_option([ClientRequest::make_request("foo", 5)], WriteOption::default())
            .await?;

        let response = results.try_next().await?.unwrap()?;
        assert_eq!(log_id(1, 0, log_index + 1), response.log_id);
        assert_eq!(ClientResponse(Some("request-2".to_string())), response.response);
        log_index += 1;

        router.wait_applied_at_least([0, 1, 2], log_index, "batch writes applied").await?;
    }

    Ok(())
}
