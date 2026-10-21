use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use openraft::Config;
use openraft::EntryPayload;
use openraft::LogIdOptionExt;
use openraft::Membership;
use openraft::Precondition;
use openraft::RPCTypes;
use openraft::RaftLogReader;
use openraft::ServerState;
use openraft::async_runtime::WatchReceiver;
use openraft::errors::ChangeMembershipError;
use openraft::errors::ClientWriteError;
use openraft::errors::ForwardReason;
use openraft::errors::ForwardToLeader;
use openraft::errors::NetworkError;
use openraft::errors::PreconditionFailed;
use openraft::errors::RPCError;
use openraft::errors::RaftError;
use openraft::errors::UncommittedLeaderLog;
use openraft::raft::ChangeMembershipRequest;
use openraft::type_config::TypeConfigExt;
use openraft::type_config::alias::LeaderIdOf;
use openraft::type_config::alias::LogIdOf;
use openraft::vote::RaftLeaderIdExt;
use openraft_memstore::MemNodeId;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::rpc_request::RpcRequest;
use crate::fixtures::ut_harness;

/// When a change-membership log is committed, the `RaftState.membership_state` should be updated.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn update_membership_state() -> anyhow::Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {3,4}).await?;

    tracing::info!(log_index, "--- change membership from 012 to 01234");
    {
        let leader = router.get_raft_handle(&0)?;
        let request = ChangeMembershipRequest::new([0, 1, 2, 3, 4], false);
        let change = leader.change_membership_with_payload(request);
        let outcome = change.await?;

        let joint = outcome.joint.as_ref().expect("voter change should enter joint consensus");
        let joint_log_index = log_index + 1;
        assert_eq!(joint_log_index, joint.log_id.index);

        let uniform_log_index = log_index + 2;
        assert_eq!(uniform_log_index, outcome.uniform.log_id.index);

        tracing::info!(
            joint_log_index,
            uniform_log_index,
            "--- inspect membership log payloads"
        );
        {
            let joint_membership = joint.membership.clone().unwrap();
            let uniform_membership = outcome.uniform.membership.clone().unwrap();
            let expected_memberships = vec![joint_membership, uniform_membership];

            let (mut log_store, _) = router.get_storage_handle(&0)?;
            let entries = log_store.try_get_log_entries(joint_log_index..=uniform_log_index).await?;
            let mut actual_memberships = Vec::new();
            for entry in entries {
                let membership = match entry.payload {
                    EntryPayload::Membership(membership) => membership,
                    payload => panic!("expected membership payload, got: {payload:?}"),
                };
                actual_memberships.push(membership);
            }

            assert_eq!(expected_memberships, actual_memberships);
        }

        log_index = uniform_log_index;

        tracing::info!(log_index, "--- change_membership blocks until success: {:?}", outcome);

        for node_id in [0, 1, 2, 3, 4] {
            router
                .wait(&node_id, timeout())
                .applied_index(Some(log_index), "change-membership log applied")
                .await?;
            router
                .external_request(node_id, move |st| {
                    tracing::debug!("--- got state: {:?}", st);
                    assert_eq!(st.membership_state.committed().log_id().index(), Some(log_index));
                    assert_eq!(st.membership_state.effective().log_id().index(), Some(log_index));
                })
                .await?;
        }
    }

    Ok(())
}

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn change_with_new_learner_blocking() -> anyhow::Result<()> {
    // Add a member without adding it as learner, in blocking mode it should finish successfully.

    let lag_threshold = 1;

    let config = Arc::new(
        Config {
            replication_lag_threshold: lag_threshold,
            enable_tick: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let mut log_index = router.new_cluster(btreeset! {0}, btreeset! {}).await?;

    tracing::info!(log_index, "--- write up to 100 logs");
    {
        router.client_request_many(0, "non_voter_add", 100 - log_index as usize).await?;
        log_index = 100;

        router.wait(&0, timeout()).applied_index(Some(log_index), "received 100 logs").await?;
    }

    tracing::info!(log_index, "--- change membership without adding-learner");
    {
        router.new_raft_node(1).await;
        router.add_learner(0, 1).await?;
        log_index += 1;
        router.wait(&0, timeout()).applied_index(Some(log_index), "add learner").await?;

        let node = router.get_raft_handle(&0)?;
        let res = node.change_membership([0, 1], false).await?;
        log_index += 1;
        tracing::info!(log_index, "--- change_membership blocks until success: {:?}", res);

        for node_id in 0..2 {
            let (mut sto, _sm) = router.get_storage_handle(&node_id)?;
            let logs = sto.try_get_log_entries(..).await?;
            assert_eq!(log_index, logs[logs.len() - 1].log_id.index(), "node: {}", node_id);
            // 0-th log
            assert_eq!(log_index + 1, logs.len() as u64, "node: {}", node_id);
        }
    }

    Ok(())
}

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn change_without_adding_learner() -> anyhow::Result<()> {
    let config = Arc::new(Config { ..Default::default() }.validate()?);
    let mut router = RaftRouter::new(config.clone());

    let log_index = router.new_cluster(btreeset! {0}, btreeset! {}).await?;
    router.wait(&0, timeout()).applied_index(Some(log_index), "received 100 logs").await?;

    router.new_raft_node(1).await;
    let leader = router.get_raft_handle(&0)?;

    tracing::info!(
        log_index,
        "--- change membership without adding-learner, allow_lagging=true"
    );
    {
        let res = leader.change_membership([0, 1], false).await;
        let raft_err = res.unwrap_err();
        tracing::debug!("raft_err: {:?}", raft_err);

        match raft_err.api_error().unwrap() {
            ClientWriteError::ChangeMembershipError(ChangeMembershipError::LearnerNotFound(err)) => {
                assert_eq!(1, err.node_id);
            }
            _ => {
                unreachable!("expect LearnerNotFound")
            }
        }
    }

    tracing::info!(log_index, "--- change membership without adding-learner");
    {
        let res = leader.change_membership([0, 1], false).await;
        let raft_err = res.unwrap_err();
        match raft_err.api_error().unwrap() {
            ClientWriteError::ChangeMembershipError(ChangeMembershipError::LearnerNotFound(err)) => {
                assert_eq!(1, err.node_id);
            }
            _ => {
                unreachable!("expect LearnerNotFound")
            }
        }
    }

    Ok(())
}

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn change_with_turn_removed_voter_to_learner() -> anyhow::Result<()> {
    // Add a member without adding it as learner, in blocking mode it should finish successfully.

    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());
    let timeout = Some(Duration::from_millis(1000));
    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {}).await?;

    tracing::info!(log_index, "--- write up to 1 logs");
    {
        router.client_request_many(0, "non_voter_add", 1).await?;
        log_index += 1;

        // all the nodes MUST recv the log
        for id in [0, 1, 2] {
            router.wait(&id, timeout).applied_index(Some(log_index), "append a log").await?;
        }
    }

    {
        let node = router.get_raft_handle(&0)?;
        node.change_membership([0, 1], true).await?;
        // 1 for change_membership
        log_index += 1;

        // all the nodes MUST recv the change_membership log
        for id in [0, 1] {
            router.wait(&id, timeout).applied_index(Some(log_index), "append a log").await?;
        }
    }

    tracing::info!(log_index, "--- write up to 1 logs");
    {
        router.client_request_many(0, "non_voter_add", 1).await?;
        log_index += 1;

        // node [0,1] MUST recv the log
        for id in [0, 1] {
            router.wait(&id, timeout).applied_index(Some(log_index), "append a log").await?;
        }

        // node 2 MUST stay in learner state and is able to receive new logs
        router
            .wait(&2, timeout)
            .metrics(
                |x| x.state == ServerState::Learner,
                &format!("n{}.state -> {:?}", 2, ServerState::Learner),
            )
            .await?;

        // node [2] MUST recv the log
        router.wait(&2, timeout).applied_index(Some(log_index), "append a log").await?;

        // check membership
        for id in [0, 1, 2] {
            router
                .wait(&id, timeout)
                .metrics(
                    |x| x.membership_config.voter_ids().collect::<BTreeSet<MemNodeId>>() == btreeset![0, 1],
                    "members: [0,1]",
                )
                .await?;
        }
    }

    Ok(())
}

/// The leader must commit a log of its own term before it changes membership, even to add a
/// learner.
///
/// Without this barrier, two leaders of different terms could each change membership from the
/// same committed parent, for example leave the joint `[{0,1,2}, {3,4,5}]` for `{0,1,2}` and for
/// `{3,4,5}`, and the two results share no node.
///
/// A valid leader lease does not imply the barrier, so this test keeps the two apart: the
/// pre-hook drops only the AppendEntries that carry entries, so the new leader's blank log never
/// reaches a quorum, while the entry-less heartbeats still get acked and keep its lease valid.
/// Heartbeats and ticking therefore stay on here.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn uncommitted_leader_log_blocks_the_change() -> anyhow::Result<()> {
    let config = Arc::new(
        Config {
            enable_elect: false,
            heartbeat_interval: 50,
            election_timeout_min: 500,
            election_timeout_max: 501,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {3}).await?;
    let old_leader = router.get_raft_handle(&0)?;
    let new_leader = router.get_raft_handle(&1)?;

    tracing::info!(log_index, "--- drop every AppendEntries that carries entries");
    {
        router
            .set_rpc_pre_hook(RPCTypes::AppendEntries, |_router, req, from, to| {
                let mut carries_entries = false;
                if let RpcRequest::AppendEntries(append) = &req {
                    carries_entries = !append.entries.is_empty();
                }

                let res = if carries_entries {
                    let msg = format!("blocked: {}->{} append-entries with entries", from, to);
                    Err(RPCError::Network(NetworkError::<TypeConfig>::from_string(msg)))
                } else {
                    Ok(())
                };
                Box::pin(futures::future::ready(res))
            })
            .await;
    }

    tracing::info!(
        log_index,
        "--- transfer leadership to node 1, whose blank log cannot commit"
    );
    {
        old_leader.trigger().transfer_leader(1).await?;
        new_leader
            .wait(timeout())
            .metrics(
                |m| {
                    let is_leader = m.state == ServerState::Leader;
                    let commit_tracking_reset = m.cluster_committed.is_none();
                    let quorum_acked = m.last_quorum_acked.is_some();

                    is_leader && commit_tracking_reset && quorum_acked
                },
                "node 1 leads with reset commit tracking and a quorum keeps acking it",
            )
            .await?;
    }

    let blocked_metrics = new_leader.metrics().borrow_watched().clone();
    let noop_log_id = {
        let leader_id = LeaderIdOf::<TypeConfig>::new_committed(blocked_metrics.current_term, 1);
        LogIdOf::<TypeConfig>::new(leader_id, blocked_metrics.last_log_index.unwrap())
    };

    tracing::info!(
        log_index,
        "--- change_membership() and add_learner() are blocked by the uncommitted blank log"
    );
    {
        let uncommitted = UncommittedLeaderLog {
            committed: None,
            leader_log_id: noop_log_id,
        };
        let want = ClientWriteError::ChangeMembershipError(ChangeMembershipError::UncommittedLeaderLog(uncommitted));

        let fu = new_leader.change_membership([0, 1, 2, 3], false);
        let res = TypeConfig::timeout(Duration::from_secs(1), fu).await;
        let err = res.expect("the uncommitted blank log rejects change_membership() at once").unwrap_err();
        assert_eq!(RaftError::APIError(want.clone()), err);

        let fu = new_leader.add_learner(3, (), false);
        let res = TypeConfig::timeout(Duration::from_secs(1), fu).await;
        let err = res.expect("the uncommitted blank log rejects add_learner() at once").unwrap_err();
        assert_eq!(RaftError::APIError(want), err);
    }

    tracing::info!(
        log_index,
        "--- a stale precondition is reported instead of the blank-log barrier"
    );
    {
        // An initialized node always has an effective membership log id, so `None` never holds.
        let stale = Precondition::LastMembershipLogId {
            last_membership_log_id: None,
        };
        let err = new_leader.change_membership_if([0, 1, 2, 3], false, [stale]).await.unwrap_err();

        let want = PreconditionFailed::LastMembershipLogIdMismatch {
            expected: None,
            actual: *blocked_metrics.membership_config.log_id(),
        };
        assert_eq!(RaftError::APIError(ClientWriteError::PreconditionFailed(want)), err);
    }

    tracing::info!(
        log_index,
        "--- an expired lease takes precedence over the blank-log barrier"
    );
    {
        router.set_network_error(1, true);
        let leader_lease = Duration::from_millis(config.election_timeout_max);
        TypeConfig::sleep(leader_lease).await;

        let forward = ForwardToLeader::empty().with_reason(ForwardReason::LeaseExpired);
        let want = ClientWriteError::ForwardToLeader(forward);
        let want = RaftError::APIError(want);

        let res = new_leader.change_membership([0, 1, 2, 3], false).await;
        let err = res.unwrap_err();
        assert_eq!(want, err);

        let res = new_leader.add_learner(3, (), false).await;
        let err = res.unwrap_err();
        assert_eq!(want, err);

        let res = new_leader.change_membership_if([0, 1, 2, 3], false, []).await;
        let err = res.unwrap_err();
        assert_eq!(want, err);

        let request = ChangeMembershipRequest::new([0, 1, 2, 3], false);
        let res = new_leader.change_membership_with_payload(request).await;
        let err = res.unwrap_err();
        assert_eq!(want, err);
    }

    tracing::info!(log_index, "--- the blocked calls wrote nothing");
    {
        let metrics = new_leader.metrics().borrow_watched().clone();
        assert_eq!(blocked_metrics.last_log_index, metrics.last_log_index);
        assert_eq!(blocked_metrics.membership_config, metrics.membership_config);
    }

    tracing::info!(log_index, "--- the same change succeeds once the blank log commits");
    {
        router.set_network_error(1, false);
        router.rpc_pre_hook(RPCTypes::AppendEntries, None).await;
        new_leader.wait(timeout()).applied_index(Some(noop_log_id.index), "blank log committed").await?;

        let resp = new_leader.change_membership([0, 1, 2, 3], false).await?;

        // Adding one voter writes a single uniform entry.
        assert_eq!(noop_log_id.index + 1, resp.log_id.index);
        let want = Membership::new_with_defaults(vec![btreeset! {0,1,2,3}], []);
        assert_eq!(Some(want), resp.membership);
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1000))
}
