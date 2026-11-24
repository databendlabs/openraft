use std::io;
use std::sync::Arc;

use anyhow::Result;
use futures::prelude::*;
use openraft::Config;
use openraft::OptionalSend;
use openraft::Raft;
use openraft::ServerState;
use openraft::StorageError;
use openraft::errors::ClientWriteError;
use openraft::errors::Fatal;
use openraft::errors::ForwardReason;
use openraft::raft::WriteOption;
use openraft::raft::WriteResponse;
use openraft::raft::WriteResult;
use openraft::type_config::TypeConfigExt;
use openraft::type_config::alias::LeaderIdOf;
use openraft::type_config::alias::LogIdOf;
use openraft::vote::RaftLeaderId;
use openraft_memstore::ClientRequest;
use openraft_memstore::ClientResponse;
use openraft_memstore::IntoMemClientRequest;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::WAIT_TIMEOUT;
use crate::fixtures::log_id;
use crate::fixtures::rpc_kind::RpcKind;
use crate::fixtures::rpc_request::RpcRequest;
use crate::fixtures::rpc_rule::RpcRule;
use crate::fixtures::timeout;
use crate::fixtures::ut_harness;

/// An item of a `Raft::client_write_many()` stream, with the log id in place of a response.
type Item = Result<Result<LogIdOf<TypeConfig>, ClientWriteError<TypeConfig>>, Fatal<TypeConfig>>;

const APPLY_ERROR: &str = "injected apply failure";

/// Read a `Raft::client_write_many()` stream to its end, keeping the log id of each write.
async fn read_to_end<S>(stream: S) -> Result<Vec<Item>>
where S: Stream<Item = Result<WriteResult<TypeConfig>, Fatal<TypeConfig>>> + OptionalSend {
    let items = stream.map(|item| item.map(|result| result.map(|response| response.log_id)));
    let items = TypeConfig::timeout(WAIT_TIMEOUT, items.collect()).await?;
    Ok(items)
}

/// Test Raft::client_write_many() for batch writes
///
/// Test that multiple writes can be submitted in a single batch and results are returned in order.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn client_write_many() -> Result<()> {
    let config = Config {
        enable_tick: false,
        enable_elect: false,
        ..Default::default()
    };

    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initializing cluster");
    let mut log_index = router.new_cluster([0, 1, 2], []).await?;

    let n0 = router.get_raft_handle(&0)?;

    // Create multiple requests to write in batch
    let requests: Vec<ClientRequest> = (0..5).map(|i| ClientRequest::make_request("batch", i)).collect();

    // Submit batch and collect results from stream
    let mut stream = n0.client_write_many(requests, WriteOption::default()).await?;

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

    tracing::info!(log_index, "--- an empty batch yields no result");
    {
        let stream = n0.client_write_many([], WriteOption::default()).await?;
        let items = read_to_end(stream).await?;
        let expected: Vec<Item> = vec![];
        assert_eq!(expected, items);
    }

    Ok(())
}

/// `Raft::client_write_many()` with `WriteOption::expect_leader()` appends a batch whose expected
/// leader matches, and yields `ForwardToLeader` for every entry of a batch whose expected leader
/// does not.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn client_write_many_expect_leader() -> Result<()> {
    let config = Config {
        enable_tick: false,
        enable_elect: false,
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
        let write_option = WriteOption::expect_leader(leader_id);
        let results: Vec<_> = n0.client_write_many(requests, write_option).await?.try_collect().await?;

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
        let write_option = WriteOption::expect_leader(fake_leader);
        let results: Vec<_> = n0.client_write_many(requests, write_option).await?.try_collect().await?;

        assert_eq!(2, results.len());
        for result in results {
            let forward = match result {
                Err(ClientWriteError::ForwardToLeader(forward)) => forward,
                Err(err) => panic!("expected ForwardToLeader, got: {:?}", err),
                Ok(response) => panic!("expected ForwardToLeader, got a write at {}", response.log_id),
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
        let mut results = n0.client_write_many([ClientRequest::make_request("foo", 5)], WriteOption::default()).await?;

        let response = results.try_next().await?.unwrap()?;
        assert_eq!(log_id(1, 0, log_index + 1), response.log_id);
        assert_eq!(ClientResponse(Some("request-2".to_string())), response.response);
        log_index += 1;

        router.wait_applied_at_least([0, 1, 2], log_index, "batch writes applied").await?;
    }

    Ok(())
}

/// A storage error that stops RaftCore before a batch is applied reaches the stream of
/// `Raft::client_write_many()`, even after another caller has joined RaftCore.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn client_write_many_storage_error() -> Result<()> {
    let config = Config {
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- bring up a single-node cluster");
    let log_index = router.new_cluster([0], []).await?;
    let n0 = router.get_raft_handle(&0)?;

    tracing::info!(
        log_index,
        "--- fail the apply of a batch: RaftCore stops with a storage error"
    );
    let (stream, fatal) = {
        let (_log_store, sm) = router.get_storage_handle(&0)?;
        sm.fail_next_apply(io::Error::other(APPLY_ERROR));

        let requests = (1..=2).map(|i| ClientRequest::make_request("foo", i));
        let stream = n0.client_write_many(requests, WriteOption::default()).await?;

        let source = io::Error::other(APPLY_ERROR);
        let error_source = TypeConfig::err_from_error(&source);
        // The batch commits at once and is applied in one call, whose error names its last entry.
        let storage_error = StorageError::apply(log_id(1, 0, log_index + 2), error_source);
        let fatal = Fatal::StorageError(storage_error);

        let metrics = n0.wait(timeout()).state(ServerState::Shutdown, "RaftCore stops").await?;
        assert_eq!(Err(fatal.clone()), metrics.running_state);
        (stream, fatal)
    };

    tracing::info!(log_index, "--- join RaftCore before the stream reads its result");
    {
        let result = n0.try_shutdown().await;
        assert_eq!(Ok(()), result);
    }

    tracing::info!(log_index, "--- the stream yields the storage error, then ends");
    {
        let items = read_to_end(stream).await?;
        let expected: Vec<Item> = vec![Err(fatal)];
        assert_eq!(expected, items);
    }

    Ok(())
}

/// When RaftCore stops before a batch is fully applied, the stream of `Raft::client_write_many()`
/// yields the results of the applied entries, then the error that stopped RaftCore, and ends.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn client_write_many_core_stopped() -> Result<()> {
    for panic in [false, true] {
        let config = Config {
            enable_tick: false,
            enable_heartbeat: false,
            enable_elect: false,
            // One entry per AppendEntries request, so that a rule can hold back the second one.
            max_payload_entries: 1,
            ..Default::default()
        };
        let mut router = RaftRouter::new(config)?;

        tracing::info!(panic, "--- bring up a 2-node cluster");
        let log_index = router.new_cluster([0, 1], []).await?;
        let n0 = router.get_raft_handle(&0)?;

        tracing::info!(panic, log_index, "--- write a batch whose second entry cannot commit");
        let mut stream = {
            let held_index = log_index + 2;
            let rule = RpcRule::new(0, 1, RpcKind::AppendEntries).matching(move |request| {
                let RpcRequest::AppendEntries(request) = request else {
                    return false;
                };
                request.entries.iter().any(|entry| entry.log_id.index() == held_index)
            });
            router.add_rpc_rule(rule);

            let requests = (1..=2).map(|i| ClientRequest::make_request("foo", i));
            n0.client_write_many(requests, WriteOption::default()).await?
        };

        tracing::info!(
            panic,
            log_index,
            "--- read the first result, then cancel a pending poll for the second"
        );
        let mut items = {
            let items = read_to_end(stream.by_ref().take(1)).await?;
            let pending = stream.next().now_or_never();
            assert!(pending.is_none(), "the second entry cannot commit");
            items
        };

        tracing::info!(panic, log_index, "--- stop RaftCore while the second result is pending");
        let fatal = if panic {
            router
                .external_request(0, |_state| {
                    panic!("injected RaftCore panic");
                })
                .await?;
            Fatal::Panicked
        } else {
            n0.shutdown().await?;
            Fatal::Stopped
        };

        tracing::info!(panic, log_index, "--- the stream yields the stop error, then ends");
        {
            items.extend(read_to_end(stream).await?);
            let expected: Vec<Item> = vec![Ok(Ok(log_id(1, 0, log_index + 1))), Err(fatal)];
            assert_eq!(expected, items);
        }
    }

    Ok(())
}

/// When the state machine drops the responders of a batch while RaftCore keeps running, the
/// stream of `Raft::client_write_many()` yields `Fatal::Stopped` once the bounded wait for a
/// RaftCore stop ends, instead of joining the running RaftCore.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn client_write_many_dropped_responder() -> Result<()> {
    let config = Config {
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- bring up a single-node cluster");
    let log_index = router.new_cluster([0], []).await?;
    let n0 = router.get_raft_handle(&0)?;

    tracing::info!(
        log_index,
        "--- the state machine worker panics while it applies a batch"
    );
    {
        let (_log_store, sm) = router.get_storage_handle(&0)?;
        sm.panic_on_apply(true);

        let requests = (1..=2).map(|i| ClientRequest::make_request("foo", i));
        let stream = n0.client_write_many(requests, WriteOption::default()).await?;
        let items = read_to_end(stream).await?;
        let expected: Vec<Item> = vec![Err(Fatal::Stopped)];
        assert_eq!(expected, items);
    }

    tracing::info!(log_index, "--- RaftCore is still running");
    {
        let server_state = n0.with_raft_state(|state| state.server_state).await?;
        assert_eq!(ServerState::Leader, server_state);
    }

    Ok(())
}

/// A stream of `Raft::client_write_many()` that the application keeps does not keep the node
/// running: dropping the last `Raft` handle stops RaftCore, and the stream yields
/// `Fatal::Stopped` and ends.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn client_write_many_kept_stream_lets_node_stop() -> Result<()> {
    let config = Config {
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- bring up a 2-node cluster");
    let log_index = router.new_cluster([0, 1], []).await?;

    tracing::info!(
        log_index,
        "--- write a batch that cannot commit, and cancel a poll of its stream"
    );
    let (stream, wait) = {
        router.set_unreachable(1, true);
        let n0 = router.get_raft_handle(&0)?;

        let requests = (1..=2).map(|i| ClientRequest::make_request("foo", i));
        let mut stream = n0.client_write_many(requests, WriteOption::default()).await?;
        n0.wait(timeout()).log_index(Some(log_index + 2), "the batch is appended").await?;

        let pending = stream.next().now_or_never();
        assert!(pending.is_none(), "the batch cannot commit");
        (stream, n0.wait(timeout()))
    };

    tracing::info!(log_index, "--- drop every handle of node 0 but keep the stream");
    {
        let node = router.detach_node(0);
        drop(node);
        wait.state(ServerState::Shutdown, "dropping the last handle stops RaftCore").await?;
    }

    tracing::info!(log_index, "--- the stream yields Fatal::Stopped, then ends");
    {
        let items = read_to_end(stream).await?;
        let expected: Vec<Item> = vec![Err(Fatal::Stopped)];
        assert_eq!(expected, items);
    }

    Ok(())
}

/// A batch submitted before `Raft::run()` stays pending, and a shutdown before `run()` makes the
/// stream of `Raft::client_write_many()` yield `Fatal::Stopped` and end.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn client_write_many_before_run() -> Result<()> {
    let config = Config {
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config.clone())?;

    tracing::info!("--- build a node without running it, and submit a batch");
    let (node, stream) = {
        let (log_store, sm) = router.new_store();
        let node = Raft::build(0, Arc::new(config), router.clone(), log_store, sm).await?;

        let requests = (1..=2).map(|i| ClientRequest::make_request("foo", i));
        let mut stream = node.client_write_many(requests, WriteOption::default()).await?;
        let pending = stream.next().now_or_never();
        assert!(pending.is_none(), "RaftCore has not started");
        (node, stream)
    };

    tracing::info!("--- shut the node down before run(); the stream yields Fatal::Stopped, then ends");
    {
        let result = node.try_shutdown().await;
        assert_eq!(Ok(()), result);

        let items = read_to_end(stream).await?;
        let expected: Vec<Item> = vec![Err(Fatal::Stopped)];
        assert_eq!(expected, items);
    }

    Ok(())
}
