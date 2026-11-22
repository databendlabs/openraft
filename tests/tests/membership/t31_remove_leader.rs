use std::time::Duration;

use anyhow::Result;
use maplit::btreemap;
use maplit::btreeset;
use openraft::ChangeMembers;
use openraft::Config;
use openraft::ReadPolicy;
use openraft::ServerState;
use openraft::StepDownPolicy;
use openraft::Vote;
use openraft::async_runtime::OneshotSender;
use openraft::errors::ClientWriteError;
use openraft::impls::ProgressResponder;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::ClientRequest;
use openraft_memstore::IntoMemClientRequest;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::WAIT_TIMEOUT;
use crate::fixtures::expect_timeout;
use crate::fixtures::log_id;
use crate::fixtures::timeout;
use crate::fixtures::ut_harness;

/// Change membership from {0,1} to {1,2,3}.
///
/// - The leader should transfer leadership and step down after the second membership log is
///   committed.
/// - Check logs on other node.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn remove_leader() -> Result<()> {
    // Setup test dependencies.
    let config = Config {
        election_timeout_min: 800,
        election_timeout_max: 1000,
        enable_heartbeat: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    let mut log_index = router.new_cluster([0, 1], [2, 3]).await?;

    // Submit a config change which adds two new nodes and removes the current leader.
    let orig_leader = router.leader().expect("expected the cluster to have a leader");
    assert_eq!(0, orig_leader, "expected original leader to be node 0");
    for id in [0, 1] {
        router.wait(&id).applied_index(Some(log_index), "add learner").await?;
    }

    let node = router.get_raft_handle(&orig_leader)?;
    node.change_membership([1, 2, 3], false).await?;
    // 2 for change_membership
    log_index += 2;

    tracing::info!(log_index, "--- old leader commits 2 membership log");
    {
        router
            .wait(&orig_leader)
            .applied_index(Some(log_index), "old leader commits 2 membership log")
            .await?;
    }

    // Another node(e.g. node-1) in the old cluster may not commit the second membership change log.
    // Because to commit the 2nd log it only need a quorum of the new cluster.

    router
        .wait(&1)
        .applied_index_at_least(Some(log_index), "node in old cluster commits at least 1 membership log")
        .await?;

    tracing::info!(log_index, "--- new cluster commits 2 membership logs");
    {
        // leader commit a new log.
        log_index += 1;
        tracing::debug!("--- expect log_index:{}", log_index);

        for id in [2, 3] {
            // There may be a log conflict when running with `single-term-leader` feature flag enabled.
            // Thus wait a little bit longer for this check
            router
                .wait_with_timeout(&id, Duration::from_millis(10_000))
                .applied_index_at_least(
                    Some(log_index),
                    "node in new cluster finally commit at least one blank leader-initialize log",
                )
                .await?;
        }
    }

    tracing::info!(log_index, "--- check term in new cluster");
    {
        for id in [1, 2, 3] {
            router
                .wait(&id)
                .metrics(
                    |x| x.current_term >= 2,
                    "new cluster has term >= 2 because of new election",
                )
                .await?;
        }
    }

    tracing::info!(log_index, "--- check state of the old leader");
    {
        let metrics = router.wait(&0).state(ServerState::Learner, "old leader steps down").await?;
        let cfg = &metrics.membership_config.membership();

        assert_eq!(metrics.current_term, 1);
        assert_eq!(metrics.last_log_index, Some(7));
        assert_eq!(metrics.last_applied, Some(log_id(1, 0, 7)));
        assert_eq!(metrics.membership_config.membership().get_joint_config().clone(), vec![
            btreeset![1, 2, 3]
        ]);
        assert_eq!(1, cfg.get_joint_config().len());
    }

    Ok(())
}

/// Change membership from {0,1} to {1,2,3}, keep node-0 as learner;
///
/// - The leader should NOT step down after joint log is committed.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn remove_leader_and_convert_to_learner() -> Result<()> {
    let config = Config {
        election_timeout_min: 800,
        election_timeout_max: 1000,
        enable_elect: false,
        enable_heartbeat: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    let mut log_index = router.new_cluster([0, 1], [2, 3]).await?;

    let old_leader = 0;

    tracing::info!(log_index, "--- change membership and retain removed node as learner");
    {
        let node = router.get_raft_handle(&old_leader)?;
        node.change_membership([1, 2, 3], true).await?;
        log_index += 2;
    }

    tracing::info!(log_index, "--- old leader commits 2 membership log");
    {
        router
            .wait(&old_leader)
            .applied_index(Some(log_index), "old leader commits 2 membership log")
            .await?;
    }

    // Another node(e.g. node-1) in the old cluster may not commit the second membership change log.
    // Because to commit the 2nd log it only need a quorum of the new cluster.

    router
        .wait(&1)
        .applied_index_at_least(
            Some(log_index - 1),
            "node in old cluster commits at least 1 membership log",
        )
        .await?;

    tracing::info!(log_index, "--- wait 1 sec, old leader(non-voter) stays as a leader");
    {
        TypeConfig::sleep(Duration::from_millis(1_000)).await;

        router
            .wait(&0)
            .state(
                ServerState::Leader,
                "old leader is not removed from membership, it is still a leader",
            )
            .await?;
    }

    Ok(())
}

/// Change membership from {0,1,2} to {1,2}: the removed leader transfers leadership and steps
/// down automatically.
///
/// Election is disabled: the new leader can only be established by transfer-leader.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn remove_leader_auto_transfer() -> Result<()> {
    let config = Config {
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    let mut log_index = router.new_cluster([0, 1, 2], []).await?;

    tracing::info!(log_index, "--- change membership 012 to 12");
    {
        let node = router.get_raft_handle(&0)?;
        node.change_membership([1, 2], false).await?;
        log_index += 1;
    }

    tracing::info!(
        log_index,
        "--- a new leader is established by transfer-leader, not by election"
    );
    {
        router
            .wait(&1)
            .metrics(
                |x| matches!(x.current_leader, Some(1) | Some(2)) && x.current_term >= 2,
                "a new leader is established by transfer-leader",
            )
            .await?;
    }

    tracing::info!(log_index, "--- the removed leader reverts to learner");
    {
        let metrics = router.wait(&0).state(ServerState::Learner, "removed leader steps down").await?;
        assert_eq!(
            metrics.current_term, 1,
            "the removed leader is not disturbed by the new election"
        );
    }

    Ok(())
}

/// Change membership from {0,1,2} to {1,2} with the automated step down disabled.
///
/// The removed leader keeps leading, until the application reverts it to a learner with
/// `Trigger::refresh_server_state()`.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn remove_leader_manual_step_down() -> Result<()> {
    let config = Config {
        enable_heartbeat: false,
        enable_elect: false,
        removed_leader_step_down: StepDownPolicy::Never,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    let mut log_index = router.new_cluster([0, 1, 2], []).await?;

    tracing::info!(log_index, "--- change membership 012 to 12");
    {
        let node = router.get_raft_handle(&0)?;
        node.change_membership([1, 2], false).await?;
        log_index += 1;
    }

    tracing::info!(log_index, "--- the removed leader keeps leading");
    {
        let res = router
            .wait_with_timeout(&0, Duration::from_millis(1_000))
            .metrics(
                |x| x.state != ServerState::Leader,
                "the removed leader leaves the Leader state",
            )
            .await;

        expect_timeout(res)?;
    }

    tracing::info!(log_index, "--- manually revert the removed leader to a learner");
    {
        let node = router.get_raft_handle(&0)?;
        node.trigger().refresh_server_state(None, None).await?;

        router.wait(&0).state(ServerState::Learner, "removed leader reverts to learner").await?;
    }

    Ok(())
}

/// Change membership from {0,1,2} to {1,2} with the automated step down disabled, then step down
/// the removed leader with a fenced `Trigger::refresh_server_state()`.
///
/// The refresh is dropped if the expected vote or the expected membership log id does not match
/// the current state; it proceeds when both match.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn remove_leader_fenced_step_down() -> Result<()> {
    let config = Config {
        enable_heartbeat: false,
        enable_elect: false,
        removed_leader_step_down: StepDownPolicy::Never,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    let mut log_index = router.new_cluster([0, 1, 2], []).await?;

    tracing::info!(log_index, "--- change membership 012 to 12");
    {
        let node = router.get_raft_handle(&0)?;
        node.change_membership([1, 2], false).await?;
        log_index += 1;
    }

    let node = router.get_raft_handle(&0)?;
    let metrics = router
        .wait(&0)
        .applied_index(Some(log_index), "the removed leader applies the membership log")
        .await?;
    let vote = metrics.vote;
    let membership_log_id =
        (*metrics.membership_config.log_id()).expect("an initialized cluster has a membership log id");

    tracing::info!(log_index, "--- mismatching expected vote: the refresh is dropped");
    {
        let mismatching = Vote::new(100, 100);
        node.trigger().refresh_server_state(Some(mismatching), None).await?;

        let res = router
            .wait_with_timeout(&0, Duration::from_millis(1_000))
            .metrics(
                |x| x.state != ServerState::Leader,
                "the removed leader leaves the Leader state",
            )
            .await;
        expect_timeout(res)?;
    }

    tracing::info!(
        log_index,
        "--- mismatching expected membership log id: the refresh is dropped"
    );
    {
        let mismatching = log_id(1, 0, 1);
        node.trigger().refresh_server_state(None, Some(mismatching)).await?;

        let res = router
            .wait_with_timeout(&0, Duration::from_millis(1_000))
            .metrics(
                |x| x.state != ServerState::Leader,
                "the removed leader leaves the Leader state",
            )
            .await;
        expect_timeout(res)?;
    }

    tracing::info!(
        log_index,
        "--- matching vote and membership log id: the refresh proceeds"
    );
    {
        node.trigger().refresh_server_state(Some(vote), Some(membership_log_id)).await?;

        router.wait(&0).state(ServerState::Learner, "removed leader reverts to learner").await?;
    }

    Ok(())
}

/// Remove leader node 0, then readmit it while its automatic transfer is queued: `RaftCore`
/// handles the readmission first and drops the stale transfer, and node 0 keeps leading.
///
/// The core waits for room in a full state-machine command channel while the step-down delay
/// passes, so the watcher queues the transfer behind the readmission.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn remove_leader_readmitted_before_queued_transfer() -> Result<()> {
    let step_down_delay_ms = 1_000;
    let step_down_delay = Duration::from_millis(step_down_delay_ms);
    let config = Config {
        enable_heartbeat: false,
        enable_elect: false,
        // A leader accepts a membership change only within its lease, `election_timeout_max`
        // after a quorum last acknowledged it: the lease must outlast the time the core is held.
        election_timeout_min: 10_000,
        election_timeout_max: 10_001,
        removed_leader_step_down: StepDownPolicy::After(step_down_delay_ms),
        state_machine_channel_size: Some(1),
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    let mut log_index = router.new_cluster([0, 1, 2], []).await?;
    let n0 = router.get_raft_handle(&0)?;

    tracing::info!(log_index, "--- change membership 012 to 12: the step-down delay starts");
    let removing_since = {
        let removing_since = TypeConfig::now();
        n0.change_membership([1, 2], false).await?;
        log_index += 1;
        removing_since
    };

    tracing::info!(
        log_index,
        "--- hold the core: pause the worker, fill its only command slot, commit a write"
    );
    let (release_1, release_2, completed) = {
        let (started_tx, started_rx) = TypeConfig::oneshot();
        let (release_1, release_rx_1) = TypeConfig::oneshot::<()>();
        n0.external_state_machine_request(move |_sm| {
            Box::pin(async move {
                started_tx.send(()).ok();
                release_rx_1.await.ok();
            })
        })
        .await?;
        started_rx.await?;

        let (release_2, release_rx_2) = TypeConfig::oneshot::<()>();
        n0.external_state_machine_request(move |_sm| {
            Box::pin(async move {
                release_rx_2.await.ok();
            })
        })
        .await?;

        let (responder, committed, completed) = ProgressResponder::new();
        let request = ClientRequest::make_request("foo", 1);
        n0.client_write_ff(request, Some(responder)).await?;
        log_index += 1;
        let committed = TypeConfig::timeout(WAIT_TIMEOUT, committed).await??;
        assert_eq!(log_id(1, 0, log_index), committed);

        (release_1, release_2, completed)
    };

    tracing::info!(
        log_index,
        "--- the core waits to send the apply and reads no API message"
    );
    let read = {
        let mut read = Box::pin(n0.with_raft_state(|_state| ()));
        let result = TypeConfig::timeout(Duration::from_millis(50), read.as_mut()).await;
        assert!(result.is_err(), "the held core handles no API message");
        read
    };

    tracing::info!(
        log_index,
        "--- queue a readmission of node 0 and an observation of the effective membership"
    );
    let (readmit, readmitted) = {
        let change = ChangeMembers::AddNodes(btreemap! {0 => ()});
        let mut readmit = Box::pin(n0.change_membership(change, false));
        let polled = futures::poll!(readmit.as_mut());
        assert!(polled.is_pending(), "the held core has not handled the readmission");

        let (tx, rx) = TypeConfig::oneshot();
        n0.external_request(move |state| {
            let contains = state.membership_state.effective().get_node(&0).is_some();
            tx.send(contains).ok();
        })
        .await?;

        let queued_before_delay = TypeConfig::now() < removing_since + step_down_delay;
        assert!(
            queued_before_delay,
            "the readmission is queued before the watcher's transfer"
        );

        (readmit, rx)
    };

    tracing::info!(
        log_index,
        "--- let the step-down delay pass: the watcher queues its transfer behind the readmission"
    );
    {
        TypeConfig::sleep_until(removing_since + step_down_delay * 2).await;
    }

    tracing::info!(
        log_index,
        "--- release the core: it handles the readmission, then drops the stale transfer"
    );
    {
        drop(release_1);
        drop(release_2);
        TypeConfig::timeout(WAIT_TIMEOUT, read).await??;

        TypeConfig::timeout(WAIT_TIMEOUT, readmit).await??;
        log_index += 1;

        let readmitted = TypeConfig::timeout(WAIT_TIMEOUT, readmitted).await??;
        assert!(
            readmitted,
            "the readmission is effective before the transfer is handled"
        );

        let response = TypeConfig::timeout(WAIT_TIMEOUT, completed).await???;
        assert_eq!(log_id(1, 0, log_index - 1), response.log_id);
    }

    tracing::info!(
        log_index,
        "--- node 0 still leads in term 1: it serves a write and a linearizable read"
    );
    {
        let metrics = router.wait(&0).applied_index(Some(log_index), "readmission applied").await?;
        assert_eq!(ServerState::Leader, metrics.state);
        assert_eq!(1, metrics.current_term);

        router.client_request(0, "foo", 2).await?;
        log_index += 1;
        n0.ensure_linearizable(ReadPolicy::ReadIndex).await?;
        router.wait(&0).applied_index(Some(log_index), "write applied").await?;
    }

    Ok(())
}

/// Change membership from {0,1,2} to {2}. Access {2} at once.
///
/// It should not respond a ForwardToLeader error that pointing to the removed leader.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn remove_leader_access_new_cluster() -> Result<()> {
    // Setup test dependencies.
    let config = Config {
        enable_heartbeat: false,
        enable_elect: false,
        // Disable the automated step down, to keep the new cluster leaderless.
        removed_leader_step_down: StepDownPolicy::Never,
        ..Default::default()
    };
    let mut router = RaftRouter::new(config)?;

    let mut log_index = router.new_cluster([0, 1, 2], []).await?;

    let orig_leader = 0;

    tracing::info!(log_index, "--- change membership 012 to 2");
    {
        let node = router.get_raft_handle(&orig_leader)?;
        node.change_membership([2], false).await?;
        // 2 change_membership logs
        log_index += 2;

        router
            .wait(&2)
            .metrics(
                |x| x.last_log_index == Some(log_index),
                "node-2 receives 2 membership logs",
            )
            .await?;
    }

    tracing::info!(log_index, "--- old leader commits 2 membership log");
    {
        router
            .wait(&orig_leader)
            .applied_index(Some(log_index), "old leader commits 2 membership log")
            .await?;
    }

    let res = router.write(2, ClientRequest::make_request("foo", 1)).await;
    match res {
        Ok(_) => {
            unreachable!("expect error");
        }
        Err(cli_err) => match cli_err.api_error().unwrap() {
            ClientWriteError::ForwardToLeader(fwd) => {
                assert!(fwd.leader_id.is_none());
                assert!(fwd.leader_node.is_none());
            }
            _ => {
                unreachable!("expect ForwardToLeader");
            }
        },
    }

    tracing::info!(log_index, "--- elect node-2, handle write");
    {
        let n2 = router.get_raft_handle(&2)?;
        n2.runtime_config().elect(true);
        n2.wait(timeout()).state(ServerState::Leader, "node-2 elect itself").await?;
        log_index += 1;

        router.write(2, ClientRequest::make_request("foo", 1)).await?;
        log_index += 1;

        n2.wait(timeout())
            .applied_index(Some(log_index), "node-2 become leader and handle write request")
            .await?;
    }

    Ok(())
}
