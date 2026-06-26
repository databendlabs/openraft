use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use openraft::Config;
use openraft::ReadPolicy;
use openraft::ServerState;
use openraft::async_runtime::WatchReceiver;
use openraft::errors::LinearizableReadError;
use openraft::raft::TransferLeaderRequest;
use openraft::type_config::TypeConfigExt;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// Test handling of transfer leader request.
///
/// Call [`handle_transfer_leader`](openraft::raft::Raft::handle_transfer_leader) on every
/// non-leader node to force establish a new leader.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn transfer_leader() -> anyhow::Result<()> {
    let config = Arc::new(
        Config {
            election_timeout_min: 150,
            election_timeout_max: 300,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let _log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    let n0 = router.get_raft_handle(&0)?;
    let n1 = router.get_raft_handle(&1)?;
    let n2 = router.get_raft_handle(&2)?;

    let metrics = n0.metrics().borrow_watched().clone();
    let leader_vote = metrics.vote;
    let last_log_id = metrics.last_applied;

    let req = TransferLeaderRequest::new(leader_vote, 2, last_log_id);

    tracing::info!("--- transfer Leader from 0 to 2");
    {
        n1.handle_transfer_leader(req.clone()).await?;
        n2.handle_transfer_leader(req.clone()).await?;

        n2.wait(timeout()).state(ServerState::Leader, "node-2 become leader").await?;
        n0.wait(timeout()).state(ServerState::Follower, "node-0 become follower").await?;
    }

    tracing::info!("--- cannot transfer Leader from 2 to 0 with an old vote");
    {
        let req = TransferLeaderRequest::new(leader_vote, 0, last_log_id);

        n0.handle_transfer_leader(req.clone()).await?;
        n1.handle_transfer_leader(req.clone()).await?;

        let n0_res = n0
            .wait(Some(Duration::from_millis(1_000)))
            .state(ServerState::Leader, "node-0 cannot become leader with old leader vote")
            .await;

        assert!(n0_res.is_err());
    }

    Ok(())
}

/// Test trigger transfer leader on the Leader.
///
/// Call [`trigger().transfer_leader()`](openraft::raft::trigger::Trigger::transfer_leader) on the
/// Leader node to force establish a new leader.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn trigger_transfer_leader() -> anyhow::Result<()> {
    let config = Arc::new(
        Config {
            election_timeout_min: 150,
            election_timeout_max: 300,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    let _log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    let n0 = router.get_raft_handle(&0)?;
    let n1 = router.get_raft_handle(&1)?;
    let n2 = router.get_raft_handle(&2)?;

    tracing::info!("--- trigger transfer Leader from 0 to 2");
    {
        n0.trigger().transfer_leader(2).await?;

        n2.wait(timeout()).state(ServerState::Leader, "node-2 become leader").await?;
        n0.wait(timeout()).state(ServerState::Follower, "node-0 become follower").await?;
        n1.wait(timeout()).state(ServerState::Follower, "node-1 become follower").await?;
    }

    Ok(())
}

/// Reproduces an issue where, after `transfer_leader` is triggered on a leader,
/// repeated `ensure_linearizable(ReadIndex)` calls on that same node prevent the
/// cluster from electing a new leader.
///
/// Mechanism:
/// - `transfer_leader` sets `transfer_to` on the source and broadcasts `TransferLeaderRequest`.
///   Recipients disable their lease, but only the designated target calls `elect()` directly; other
///   voters must wait for their election timeout to fire.
/// - `ensure_linearizable(ReadIndex)` on the source emits `AppendEntries`, which bumps
///   `last_update` on every reachable follower regardless of lease state. Under sustained read
///   load, the follower's election timeout is perpetually postponed.
///
/// Setup:
/// - 3-voter cluster, n0 leader.
/// - n1 (the transfer target) is fully isolated, so `TransferLeaderRequest` never reaches it and it
///   cannot vote in any subsequent election.
/// - A background task drives `ensure_linearizable(ReadIndex)` on n0 in a tight loop, simulating an
///   application that keeps issuing reads during the transfer.
///
/// After the fix, reads on the source must respect `transfer_to` and return
/// `ForwardToLeader(target)`, which (a) pins the contract behaviorally and
/// (b) stops the read-driven `AppendEntries` so n2's lease/election timer
/// can fire.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn transfer_leader_with_dead_target_does_not_block_election() -> anyhow::Result<()> {
    let election_timeout_max = 300;

    let config = Arc::new(
        Config {
            // The only AE traffic should come from ReadIndex; otherwise heartbeat AE
            // would also bump follower last_update and the bug is no longer attributable
            // to ReadIndex specifically.
            enable_heartbeat: false,
            election_timeout_min: 150,
            election_timeout_max,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());
    router.network_send_delay(0);

    tracing::info!("--- initializing cluster");
    let _log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    let n0 = router.get_raft_handle(&0)?;
    let n2 = router.get_raft_handle(&2)?;

    tracing::info!("--- isolate n1: it cannot receive TransferLeaderRequest or vote");
    router.set_network_error(1, true);

    tracing::info!("--- background task: drive ReadIndex on n0 in a tight loop");
    let r = router.clone();
    TypeConfig::spawn(async move {
        loop {
            let _ = r.ensure_linearizable(0, ReadPolicy::ReadIndex).await;
            TypeConfig::sleep(Duration::from_millis(20)).await;
        }
    });

    tracing::info!("--- trigger transfer to n1");
    n0.trigger().transfer_leader(1).await?;

    tracing::info!("--- ensure_linearizable on n0 must return ForwardToLeader(1) after transfer");
    {
        // The transfer command is processed asynchronously by RaftCore; poll briefly.
        let mut got = false;
        for _ in 0..20 {
            let res = n0.ensure_linearizable(ReadPolicy::ReadIndex).await;
            if let Err(e) = &res
                && let Some(LinearizableReadError::ForwardToLeader(fwd)) = e.api_error()
                && fwd.leader_id == Some(1)
            {
                got = true;
                break;
            }
            TypeConfig::sleep(Duration::from_millis(10)).await;
        }
        assert!(got, "expected ForwardToLeader(1) within 200ms after transfer");
    }

    tracing::info!("--- n2 must elect within 5 * election_timeout_max despite background reads");
    n2.wait(Some(Duration::from_millis(5 * election_timeout_max)))
        .state(ServerState::Leader, "n2 elects after n0 stops emitting read-index AE")
        .await?;

    Ok(())
}

/// `ensure_linearizable(LeaseRead)` on a transferring leader must redirect.
///
/// Mechanism is different from the `ReadIndex` case:
/// - `LeaseRead` short-circuits when `last_quorum_acked_time + leader_lease > now` and returns the
///   cached read without any quorum confirmation.
/// - `handle_transfer_leader` disables the *vote* lease but does not clear
///   `last_quorum_acked_time`. So pre-fix, the source keeps returning a successful `LeaseRead`
///   while a new leader may already be committing fresh entries -- the classic stale-read scenario.
///
/// After the fix, the read path checks `transfer_to` regardless of policy and returns
/// `ForwardToLeader(target)`.
///
/// The transfer target (n1) is isolated, mirroring the `ReadIndex` test. If it were
/// reachable it would elect immediately (the target calls `elect()` directly on
/// receiving `TransferLeaderRequest`), n0 would demote to follower, and the assertion
/// would pass for the wrong reason -- a follower's `forward_to_leader` always points
/// at the current leader independent of the transfer-gate behavior.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn transfer_leader_blocks_lease_read() -> anyhow::Result<()> {
    let config = Arc::new(
        Config {
            // Large lease so `last_quorum_acked + leader_lease` stays in the future for the
            // whole test. If the lease lapses, pre-fix `LeaseRead` returns
            // `ForwardToLeader::empty` for the wrong reason and masks the bug.
            heartbeat_interval: 1000,
            election_timeout_min: 1001,
            election_timeout_max: 1002,
            enable_heartbeat: false,
            enable_elect: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());
    router.network_send_delay(0);

    tracing::info!("--- initializing cluster");
    router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    let n0 = router.get_raft_handle(&0)?;

    // Refresh quorum-acked time before isolating n1, so the lease is comfortably fresh
    // and `LeaseRead` would succeed if the gate were absent.
    n0.trigger().heartbeat().await?;
    n0.wait(Some(Duration::from_millis(500)))
        .metrics(|m| m.last_quorum_acked.is_some(), "leader has fresh last_quorum_acked")
        .await?;

    // Sanity: LeaseRead succeeds before the transfer.
    n0.ensure_linearizable(ReadPolicy::LeaseRead).await.expect("LeaseRead succeeds on healthy leader");

    tracing::info!("--- isolate n1: it cannot receive TransferLeaderRequest or elect");
    router.set_network_error(1, true);

    tracing::info!("--- trigger transfer to n1");
    n0.trigger().transfer_leader(1).await?;

    tracing::info!("--- ensure_linearizable(LeaseRead) on n0 must return ForwardToLeader(1)");
    let mut got = false;
    for _ in 0..20 {
        let res = n0.ensure_linearizable(ReadPolicy::LeaseRead).await;
        if let Err(e) = &res
            && let Some(LinearizableReadError::ForwardToLeader(fwd)) = e.api_error()
            && fwd.leader_id == Some(1)
        {
            got = true;
            break;
        }
        TypeConfig::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        got,
        "expected ForwardToLeader(1) for LeaseRead within 200ms after transfer"
    );

    Ok(())
}

/// A leadership-transfer election succeeds while the other voters still hold a fresh leader
/// lease.
///
/// The `TransferLeaderRequest` is delivered only to the target: the other voters never receive
/// it, so their leases are never disabled. They grant the vote anyway, because the vote request
/// of a leadership-transfer election overrides the lease (Raft dissertation, section 4.2.3).
/// This is the deterministic form of a race where the target's vote request overtakes the
/// `TransferLeaderRequest` broadcast on its way to another voter.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn transfer_leader_overrides_lease() -> anyhow::Result<()> {
    let config = Arc::new(
        Config {
            // A lease far longer than the test: the voters' leases stay fresh throughout,
            // so a granted vote can only result from the leadership-transfer override.
            election_timeout_min: 10_000,
            election_timeout_max: 10_001,
            enable_heartbeat: false,
            enable_elect: false,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing cluster");
    router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    let n0 = router.get_raft_handle(&0)?;
    let n2 = router.get_raft_handle(&2)?;

    let metrics = n0.metrics().borrow_watched().clone();
    let leader_vote = metrics.vote;
    let last_log_id = metrics.last_applied;

    tracing::info!("--- transfer Leader from 0 to 2, deliver the request only to the target");
    {
        let req = TransferLeaderRequest::new(leader_vote, 2, last_log_id);
        n2.handle_transfer_leader(req).await?;

        n2.wait(timeout()).state(ServerState::Leader, "node-2 becomes leader within the lease").await?;
        n0.wait(timeout()).state(ServerState::Follower, "node-0 steps down").await?;
    }

    Ok(())
}

/// A transfer target may be a voter in the leader's effective membership while still
/// being a learner in its own effective membership if it has not replicated the
/// promotion entry yet.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn transfer_leader_to_promoted_learner_without_promotion_log_does_not_panic() -> anyhow::Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            enable_tick: false,
            election_timeout_min: 100,
            election_timeout_max: 200,
            ..Default::default()
        }
        .validate()?,
    );

    let mut router = RaftRouter::new(config.clone());
    router.network_send_delay(0);

    tracing::info!("--- initialize 3 voters and node 3 as a learner");
    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {3}).await?;

    let n0 = router.get_raft_handle(&0)?;
    let n3 = router.get_raft_handle(&3)?;

    router
        .wait(&3, timeout())
        .metrics(
            |m| {
                m.state == ServerState::Learner
                    && !m.membership_config.voter_ids().any(|id| id == 3)
                    && m.last_log_index == Some(log_index)
            },
            "node 3 starts as a caught-up learner",
        )
        .await?;

    tracing::info!("--- isolate node 3 before promoting it to voter");
    router.set_network_error(3, true);

    n0.change_membership([0, 1, 2, 3], false).await?;
    log_index += 2;

    router
        .wait(&0, timeout())
        .metrics(
            |m| {
                m.membership_config.voter_ids().any(|id| id == 3)
                    && m.last_applied.as_ref().map(|log_id| log_id.index()) == Some(log_index)
            },
            "leader sees node 3 as a voter",
        )
        .await?;

    router
        .wait(&3, timeout())
        .metrics(
            |m| !m.membership_config.voter_ids().any(|id| id == 3) && m.last_log_index < Some(log_index),
            "node 3 has not received its promotion log",
        )
        .await?;

    let leader_metrics = n0.metrics().borrow_watched().clone();
    let req = TransferLeaderRequest::new(leader_metrics.vote, 3, leader_metrics.last_applied);

    tracing::info!("--- deliver transfer directly to the stale target");
    n3.handle_transfer_leader(req).await?;

    TypeConfig::sleep(Duration::from_millis(50)).await;

    let alive = n3.is_initialized().await;
    assert!(
        alive.is_ok(),
        "stale transfer target should ignore the transfer without panicking: {:?}",
        alive
    );

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(500))
}
