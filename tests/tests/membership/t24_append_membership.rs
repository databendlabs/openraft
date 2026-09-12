use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::EntryPayload;
use openraft::Membership;
use openraft::Precondition;
use openraft::RPCTypes;
use openraft::ServerState;
use openraft::async_runtime::WatchReceiver;
use openraft::errors::ChangeMembershipError;
use openraft::errors::ClientWriteError;
use openraft::errors::ForwardToLeader;
use openraft::errors::NetworkError;
use openraft::errors::PreconditionFailed;
use openraft::errors::RPCError;
use openraft::errors::RaftError;
use openraft::errors::UncommittedLeaderLog;
use openraft::errors::UnsupportedMembershipTransition;
use openraft::type_config::alias::LeaderIdOf;
use openraft::type_config::alias::LogIdOf;
use openraft::vote::RaftLeaderIdExt;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::rpc_request::RpcRequest;
use crate::fixtures::ut_harness;

/// Election, heartbeat and ticking stay off: every log this test counts must come from its own
/// `append_membership()` calls.
fn config() -> Result<Arc<Config>> {
    let config = Config {
        enable_tick: false,
        enable_heartbeat: false,
        enable_elect: false,
        ..Default::default()
    }
    .validate()?;
    Ok(Arc::new(config))
}

/// Adding one voter and removing one voter each write exactly one membership entry.
///
/// `change_membership()` writes two entries for the same transition: a joint config and then a
/// uniform config.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn add_and_remove_one_voter_write_one_entry_each() -> Result<()> {
    let mut router = RaftRouter::new(config()?);

    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {3}).await?;
    let leader = router.get_raft_handle(&0)?;

    tracing::info!(log_index, "--- promote learner 3 to voter in one entry");
    {
        let proposed = Membership::new_with_defaults(vec![btreeset! {0,1,2,3}], []);
        let resp = leader.append_membership(proposed.clone(), EntryPayload::Blank, []).await?;
        log_index += 1;

        assert_eq!(log_index, resp.log_id.index);
        assert_eq!(Some(proposed), resp.membership);

        for node_id in [0, 1, 2, 3] {
            router.wait(&node_id, timeout()).applied_index(Some(log_index), "voter 3 added").await?;
        }
    }

    tracing::info!(log_index, "--- remove voter 3 in one entry");
    {
        let proposed = Membership::new_with_defaults(vec![btreeset! {0,1,2}], []);
        let resp = leader.append_membership(proposed.clone(), EntryPayload::Blank, []).await?;
        log_index += 1;

        assert_eq!(log_index, resp.log_id.index);
        assert_eq!(Some(proposed), resp.membership);

        for node_id in [0, 1, 2] {
            router.wait(&node_id, timeout()).applied_index(Some(log_index), "voter 3 removed").await?;
        }
    }

    Ok(())
}

/// Replacing one voter with another is outside the direct-append rule, and writes no log.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn unsupported_transition_writes_no_log() -> Result<()> {
    let mut router = RaftRouter::new(config()?);

    let log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {3}).await?;
    let leader = router.get_raft_handle(&0)?;

    let membership_before = leader.metrics().borrow_watched().membership_config.clone();

    tracing::info!(log_index, "--- swapping voter 0 for voter 3 is rejected");
    {
        let proposed = Membership::new_with_defaults(vec![btreeset! {1,2,3}], []);
        let err = leader.append_membership(proposed, EntryPayload::Blank, []).await.unwrap_err();

        let transition = UnsupportedMembershipTransition {
            previous: vec![btreeset! {0,1,2}],
            proposed: vec![btreeset! {1,2,3}],
        };
        let want =
            ClientWriteError::ChangeMembershipError(ChangeMembershipError::UnsupportedMembershipTransition(transition));
        assert_eq!(RaftError::APIError(want), err);
    }

    tracing::info!(log_index, "--- the rejected append wrote nothing");
    {
        let metrics = leader.metrics().borrow_watched().clone();
        assert_eq!(Some(log_index), metrics.last_log_index);
        assert_eq!(membership_before, metrics.membership_config);
    }

    Ok(())
}

/// A caller-built joint membership is stored exactly as given, and left again in one entry.
///
/// The joint membership below has three voter sets, which shows that the shared-set rule imposes
/// no maximum. Both transitions share the voter set `{0,1,2}` exactly.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn joint_membership_is_entered_and_left_in_one_entry_each() -> Result<()> {
    let mut router = RaftRouter::new(config()?);

    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {3,4}).await?;
    let leader = router.get_raft_handle(&0)?;

    tracing::info!(log_index, "--- enter a three-set joint membership in one entry");
    {
        let configs = vec![btreeset! {0,1,2}, btreeset! {2,3,4}, btreeset! {0,3,4}];
        let proposed = Membership::new_with_defaults(configs, []);
        let resp = leader.append_membership(proposed.clone(), EntryPayload::Blank, []).await?;
        log_index += 1;

        assert_eq!(log_index, resp.log_id.index);
        assert_eq!(Some(proposed), resp.membership);

        for node_id in [0, 1, 2, 3, 4] {
            router.wait(&node_id, timeout()).applied_index(Some(log_index), "joint config applied").await?;
        }
    }

    tracing::info!(
        log_index,
        "--- leave the joint membership for its shared voter set in one entry"
    );
    {
        let proposed = Membership::new_with_defaults(vec![btreeset! {0,1,2}], []);
        let resp = leader.append_membership(proposed.clone(), EntryPayload::Blank, []).await?;
        log_index += 1;

        assert_eq!(log_index, resp.log_id.index);
        assert_eq!(Some(proposed), resp.membership);

        for node_id in [0, 1, 2] {
            router.wait(&node_id, timeout()).applied_index(Some(log_index), "uniform config applied").await?;
        }
    }

    Ok(())
}

/// `Precondition::LastMembershipLogId` permits the append at the log id it was read at, and
/// rejects it once another membership entry has moved that log id.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn membership_log_id_precondition_guards_the_append() -> Result<()> {
    let mut router = RaftRouter::new(config()?);

    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {3}).await?;
    let leader = router.get_raft_handle(&0)?;

    let observed_log_id = *leader.metrics().borrow_watched().membership_config.log_id();

    tracing::info!(log_index, "--- the append guarded by the observed log id is accepted");
    {
        let precondition = Precondition::LastMembershipLogId {
            last_membership_log_id: observed_log_id,
        };
        let proposed = Membership::new_with_defaults(vec![btreeset! {0,1,2,3}], []);
        let resp = leader.append_membership(proposed, EntryPayload::Blank, [precondition]).await?;
        log_index += 1;

        assert_eq!(log_index, resp.log_id.index);
    }

    let current_log_id = *leader.metrics().borrow_watched().membership_config.log_id();

    tracing::info!(log_index, "--- a second append guarded by the same log id is rejected");
    {
        let precondition = Precondition::LastMembershipLogId {
            last_membership_log_id: observed_log_id,
        };
        let proposed = Membership::new_with_defaults(vec![btreeset! {0,1,2}], []);
        let err = leader.append_membership(proposed, EntryPayload::Blank, [precondition]).await.unwrap_err();

        let want = PreconditionFailed::LastMembershipLogIdMismatch {
            expected: observed_log_id,
            actual: current_log_id,
        };
        assert_eq!(RaftError::APIError(ClientWriteError::PreconditionFailed(want)), err);
    }

    tracing::info!(log_index, "--- the rejected append wrote nothing");
    {
        let metrics = leader.metrics().borrow_watched().clone();
        assert_eq!(Some(log_index), metrics.last_log_index);
        assert_eq!(current_log_id, *metrics.membership_config.log_id());
    }

    Ok(())
}

/// A follower answers `ForwardToLeader`; it never appends a membership entry of its own.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn follower_answers_forward_to_leader() -> Result<()> {
    let mut router = RaftRouter::new(config()?);

    let log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {3}).await?;
    let follower = router.get_raft_handle(&1)?;

    // `new_cluster()` waits for the learner to receive the last entry, not for the voters, so
    // follower 1 may still be one entry behind here. Pin its log before the append, so that the
    // last phase can tell an unchanged log from one the append grew.
    router
        .wait(&1, timeout())
        .metrics(
            |m| m.last_log_index == Some(log_index),
            "follower 1 receives every entry the cluster wrote",
        )
        .await?;

    tracing::info!(log_index, "--- a follower forwards the append to the leader");
    {
        let proposed = Membership::new_with_defaults(vec![btreeset! {0,1,2,3}], []);
        let err = follower.append_membership(proposed, EntryPayload::Blank, []).await.unwrap_err();

        let want = ClientWriteError::ForwardToLeader(ForwardToLeader::new(0, ()));
        assert_eq!(RaftError::APIError(want), err);
    }

    tracing::info!(log_index, "--- the forwarded append wrote nothing");
    {
        let metrics = follower.metrics().borrow_watched().clone();
        assert_eq!(Some(log_index), metrics.last_log_index);
    }

    Ok(())
}

/// The leader must commit a log of its own term before it appends a membership entry.
///
/// A valid leader lease does not imply that condition, so this test keeps the two apart: the
/// pre-hook drops only the AppendEntries that carry entries, so the new leader's blank log never
/// reaches a quorum, while the entry-less heartbeats still get acked and keep its lease valid.
/// Heartbeats and ticking therefore stay on here, unlike in the other tests of this file.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn uncommitted_leader_log_blocks_the_append() -> Result<()> {
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
                |m| m.state == ServerState::Leader && m.last_quorum_acked.is_some(),
                "node 1 leads and a quorum keeps acking it",
            )
            .await?;
    }

    let blocked_metrics = new_leader.metrics().borrow_watched().clone();
    let noop_log_id = {
        let leader_id = LeaderIdOf::<TypeConfig>::new_committed(blocked_metrics.current_term, 1);
        LogIdOf::<TypeConfig>::new(leader_id, blocked_metrics.last_log_index.unwrap())
    };
    let proposed = Membership::new_with_defaults(vec![btreeset! {0,1,2,3}], []);

    tracing::info!(log_index, "--- the append is blocked by the uncommitted blank log");
    {
        let err = new_leader.append_membership(proposed.clone(), EntryPayload::Blank, []).await.unwrap_err();

        let uncommitted = UncommittedLeaderLog {
            committed: None,
            leader_log_id: noop_log_id,
        };
        let want = ClientWriteError::ChangeMembershipError(ChangeMembershipError::UncommittedLeaderLog(uncommitted));
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
        let err = new_leader.append_membership(proposed.clone(), EntryPayload::Blank, [stale]).await.unwrap_err();

        let want = PreconditionFailed::LastMembershipLogIdMismatch {
            expected: None,
            actual: *blocked_metrics.membership_config.log_id(),
        };
        assert_eq!(RaftError::APIError(ClientWriteError::PreconditionFailed(want)), err);
    }

    tracing::info!(log_index, "--- the blocked append wrote nothing");
    {
        let metrics = new_leader.metrics().borrow_watched().clone();
        assert_eq!(blocked_metrics.last_log_index, metrics.last_log_index);
        assert_eq!(blocked_metrics.membership_config, metrics.membership_config);
    }

    tracing::info!(log_index, "--- the same append succeeds once the blank log commits");
    {
        router.rpc_pre_hook(RPCTypes::AppendEntries, None).await;
        new_leader.wait(timeout()).applied_index(Some(noop_log_id.index), "blank log committed").await?;

        let resp = new_leader.append_membership(proposed.clone(), EntryPayload::Blank, []).await?;

        assert_eq!(noop_log_id.index + 1, resp.log_id.index);
        assert_eq!(Some(proposed), resp.membership);
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1000))
}
