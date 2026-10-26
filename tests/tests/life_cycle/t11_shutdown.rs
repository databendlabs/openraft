use std::sync::Arc;

use anyhow::Result;
use openraft::Config;
use openraft::ConfigError;
use openraft::ServerState;
use openraft::async_runtime::WatchReceiver;
use openraft::errors::Fatal;
use openraft::errors::Infallible;
use openraft::errors::RPCError;

use crate::fixtures::Node;
use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// Shutdown raft node and check the metrics change.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn shutdown() -> Result<()> {
    let config = Config {
        enable_heartbeat: false,
        ..Default::default()
    };

    tracing::info!("--- reject invalid configuration before creating a router");
    {
        let invalid = Config {
            election_timeout_min: 1000,
            election_timeout_max: 700,
            ..Default::default()
        };
        let error = RaftRouter::new(invalid).err().expect("invalid config must fail");
        let error = error.downcast::<ConfigError>()?;
        assert_eq!(ConfigError::ElectionTimeout { min: 1000, max: 700 }, error);
    }
    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- reject invalid cluster sets without creating any nodes");
    {
        for (voters, learners, expected) in [
            (vec![1], vec![], "cluster voters must include node 0"),
            (vec![0, 1], vec![1, 2], "cluster voters and learners must be disjoint"),
        ] {
            let error = router.new_cluster(voters, learners).await.unwrap_err();
            let message = error.to_string();
            assert_eq!(expected, message);
            let metrics = router.latest_metrics();
            let ids = metrics.iter().map(|m| m.id);
            let ids: Vec<_> = ids.collect();
            let expected_ids = Vec::<u64>::new();
            assert_eq!(expected_ids, ids);
        }
        router.new_raft_node(9).await?;
        let error = router.new_cluster([0, 9], []).await.unwrap_err();
        let message = error.to_string();
        assert_eq!("node 9 is already registered", message);
        let metrics = router.latest_metrics();
        let ids = metrics.iter().map(|m| m.id);
        let ids: Vec<_> = ids.collect();
        let expected_ids = vec![9];
        assert_eq!(expected_ids, ids);
        router.stop_node(9).await?;
    }

    let _log_index = router.new_cluster([0, 1, 2], []).await?;

    tracing::info!("--- duplicate IDs must preserve the registered node");
    {
        let result = router.new_raft_node(0).await;
        let error = result.unwrap_err();
        let message = error.to_string();
        assert_eq!("node 0 is already registered", message);
    }

    tracing::info!("--- stop nodes while preserving their storage and network faults");
    {
        for i in [0, 1, 2] {
            let (log_store, state_machine) = router.get_storage_handle(&i)?;
            router.set_unreachable(i, true);
            let node = router.stop_node(i).await?;
            let same_log = Arc::ptr_eq(&log_store, &node.log_store);
            assert!(same_log);
            let same_sm = Arc::ptr_eq(&state_machine, &node.state_machine);
            assert!(same_sm);
            let result = router.emit_rpc_error::<Infallible>(3, i);
            let error = result.unwrap_err();
            let unreachable = matches!(error, RPCError::Unreachable(_));
            assert!(unreachable);
            let m = node.raft.metrics();
            assert_eq!(ServerState::Shutdown, m.borrow_watched().state, "shutdown node-{}", i);
        }
    }

    tracing::info!("--- a detached node remains available to direct API calls");
    {
        router.new_raft_node(3).await?;
        let detached = router.detach_node(3);
        let Node { raft: node, .. } = detached.unwrap();
        let state = node.with_raft_state(|state| state.server_state).await?;
        assert_eq!(ServerState::Learner, state);
        node.shutdown().await?;
    }

    Ok(())
}

/// A panicked RaftCore should also return a proper error the next time accessing the `Raft`.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn return_error_after_panic() -> Result<()> {
    let config = Config {
        enable_heartbeat: false,
        ..Default::default()
    };

    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initializing cluster");
    let log_index = router.new_cluster([0], []).await?;
    let _ = log_index; // unused;

    tracing::info!(log_index, "--- panic the RaftCore");
    {
        router
            .external_request(0, |_s| {
                panic!("foo");
            })
            .await?;
    }

    tracing::info!(
        log_index,
        "--- calls the panicked raft should get a Fatal::Panicked error"
    );
    {
        let res = router.client_request(0, "foo", 2).await;
        let err = res.unwrap_err();
        assert_eq!(Fatal::Panicked, err.into_fatal().unwrap());
    }

    Ok(())
}

/// A state-machine worker that dies while RaftCore stays alive must not hang callers.
///
/// `get_snapshot()` sends a command straight to the state-machine worker's channel, bypassing
/// RaftCore. When the worker has panicked, resolving the stop cause must fall back to
/// `Fatal::Stopped` within the bounded wait, instead of joining the still-running core forever.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn get_snapshot_returns_stopped_when_sm_worker_dies() -> Result<()> {
    let config = Config {
        enable_heartbeat: false,
        ..Default::default()
    };

    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initializing single-node cluster");
    let _log_index = router.new_cluster([0], []).await?;

    tracing::info!("--- arm the state machine to panic on the next apply");
    {
        let (_log_store, sm) = router.get_storage_handle(&0)?;
        sm.panic_on_apply(true);
    }

    tracing::info!("--- a client write makes the SM worker panic and die; RaftCore stays up");
    {
        let res = router.client_request(0, "c", 1).await;
        assert!(
            res.is_err(),
            "write must fail: the SM worker panicked while applying it"
        );
    }

    tracing::info!("--- get_snapshot() must resolve to Fatal::Stopped, not hang");
    {
        let raft = router.get_raft_handle(&0)?;
        let err = raft.get_snapshot().await.unwrap_err();
        assert_eq!(Fatal::Stopped, err.into_fatal().unwrap());
    }

    Ok(())
}

/// `with_state_machine` must not hang when its function panics the state-machine worker.
///
/// The user function runs in the SM worker; a panic there kills the worker and drops the
/// responder while RaftCore keeps running. Resolving the stop cause must fall back to
/// `Fatal::Stopped` within the bounded wait instead of joining the still-running core forever.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn with_state_machine_returns_stopped_when_func_panics() -> Result<()> {
    let config = Config {
        enable_heartbeat: false,
        ..Default::default()
    };

    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initializing single-node cluster");
    let _log_index = router.new_cluster([0], []).await?;

    tracing::info!("--- a panicking with_state_machine func kills the worker; the call must return Fatal::Stopped");
    {
        let raft = router.get_raft_handle(&0)?;
        let res = raft
            .with_state_machine::<_, ()>(|_sm| Box::pin(async move { panic!("injected worker panic") }))
            .await;
        assert_eq!(Fatal::Stopped, res.unwrap_err());
    }

    Ok(())
}

/// After shutdown(), access to Raft should return a Fatal::Stopped error.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn return_error_after_shutdown() -> Result<()> {
    let config = Config {
        enable_heartbeat: false,
        ..Default::default()
    };

    let mut router = RaftRouter::new(config)?;

    tracing::info!("--- initializing cluster");
    let log_index = router.new_cluster([0], []).await?;
    let _ = log_index; // unused;

    tracing::info!(log_index, "--- shutdown the raft");
    {
        let n = router.get_raft_handle(&0)?;
        n.shutdown().await?;
    }

    tracing::info!(
        log_index,
        "--- calls the panicked raft should get a Fatal::Panicked error"
    );
    {
        let res = router.client_request(0, "foo", 2).await;
        let err = res.unwrap_err();
        assert_eq!(Fatal::Stopped, err.into_fatal().unwrap());
    }

    Ok(())
}
