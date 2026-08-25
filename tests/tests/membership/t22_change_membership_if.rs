use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::Config;
use openraft::EntryPayload;
use openraft::Precondition;
use openraft::async_runtime::WatchReceiver;
use openraft::errors::ClientWriteError;
use openraft::errors::ForwardToLeader;
use openraft::errors::PreconditionFailed;
use openraft::errors::RaftError;
use openraft::raft::ChangeMembershipRequest;
use openraft::type_config::alias::LeaderIdOf;
use openraft::type_config::alias::LogIdOf;
use openraft::vote::RaftLeaderIdExt;
use openraft_memstore::TypeConfig;

use crate::fixtures::RaftRouter;
use crate::fixtures::ut_harness;

/// A `LastMembershipLogId` precondition that matches must survive both steps of a joint change.
///
/// The second step of `change_membership` proposes the uniform config after the joint config is
/// committed, at which point the caller's membership log id is already stale. It must not be
/// carried into that step.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn matching_membership_log_id_completes_joint_change() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {3}).await?;
    let leader = router.get_raft_handle(&0)?;

    let membership_log_id = {
        let metrics = leader.metrics().borrow_watched().clone();
        *metrics.membership_config.log_id()
    };

    tracing::info!(
        log_index,
        "--- add voter 3 guarded by the membership log id it was read at"
    );
    {
        let precondition = Precondition::LastMembershipLogId {
            last_membership_log_id: membership_log_id,
        };
        let request = ChangeMembershipRequest::<TypeConfig>::new([0, 1, 2, 3], false)
            .with_payload(EntryPayload::Blank, EntryPayload::Blank)
            .with_preconditions([precondition]);
        let change = leader.change_membership_with_payload(request);
        let outcome = change.await?;
        let resp = outcome.uniform.as_ref().expect("voter change should enter joint consensus");

        // A joint config log and a uniform config log.
        log_index += 2;

        let voters = resp.membership.as_ref().unwrap().voter_ids().collect::<std::collections::BTreeSet<_>>();
        assert_eq!(btreeset! {0,1,2,3}, voters);
        assert_eq!(1, resp.membership.as_ref().unwrap().get_joint_config().len());
    }

    tracing::info!(log_index, "--- every node applies both membership logs");
    {
        for node_id in [0, 1, 2, 3] {
            router.wait(&node_id, timeout()).applied_index(Some(log_index), "uniform config applied").await?;
        }
    }

    Ok(())
}

/// A `LastMembershipLogId` precondition that no longer matches rejects the change.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn stale_membership_log_id_rejects_change() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {3}).await?;
    let leader = router.get_raft_handle(&0)?;

    let stale_log_id = {
        let metrics = leader.metrics().borrow_watched().clone();
        *metrics.membership_config.log_id()
    };

    tracing::info!(
        log_index,
        "--- another membership change moves the effective membership"
    );
    {
        leader.change_membership([0, 1, 2, 3], false).await?;
        log_index += 2;
    }

    let current_log_id = {
        let metrics = leader.metrics().borrow_watched().clone();
        *metrics.membership_config.log_id()
    };

    tracing::info!(log_index, "--- the change guarded by the stale log id is rejected");
    {
        let precondition = Precondition::LastMembershipLogId {
            last_membership_log_id: stale_log_id,
        };
        let err = leader.change_membership_if([0, 1, 2], false, [precondition]).await.unwrap_err();

        let want = PreconditionFailed::LastMembershipLogIdMismatch {
            expected: stale_log_id,
            actual: current_log_id,
        };
        assert_eq!(RaftError::APIError(ClientWriteError::PreconditionFailed(want)), err);
    }

    tracing::info!(log_index, "--- the rejected change wrote nothing");
    {
        let metrics = leader.metrics().borrow_watched().clone();
        assert_eq!(current_log_id, *metrics.membership_config.log_id());
        assert_eq!(Some(log_index), metrics.last_log_index);
    }

    Ok(())
}

/// A `CommittedLeaderId` precondition passes for the established leader and fails for any other.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn committed_leader_id_guards_the_change() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {3}).await?;
    let leader = router.get_raft_handle(&0)?;

    let established = {
        let metrics = leader.metrics().borrow_watched().clone();
        LeaderIdOf::<TypeConfig>::new_committed(metrics.current_term, metrics.current_leader.unwrap())
    };
    // `leader_id_std` discards the node id, so the mismatching leader id must differ by term.
    let other = LeaderIdOf::<TypeConfig>::new_committed(100, 2);

    tracing::info!(log_index, "--- a change guarded by another leader id is rejected");
    {
        let precondition = Precondition::CommittedLeaderId {
            committed_leader_id: other,
        };
        let err = leader.change_membership_if([0, 1, 2, 3], false, [precondition]).await.unwrap_err();

        let want = PreconditionFailed::CommittedLeaderIdMismatch {
            expected: other,
            actual: Some(established),
        };
        assert_eq!(RaftError::APIError(ClientWriteError::PreconditionFailed(want)), err);
    }

    tracing::info!(
        log_index,
        "--- a change guarded by the established leader id is accepted"
    );
    {
        let precondition = Precondition::CommittedLeaderId {
            committed_leader_id: established,
        };
        leader.change_membership_if([0, 1, 2, 3], false, [precondition]).await?;
        log_index += 2;

        router.wait(&0, timeout()).applied_index(Some(log_index), "uniform config applied").await?;
    }

    Ok(())
}

/// A `LastLogId` precondition passes at the current last log id and fails at any other.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn last_log_id_guards_the_change() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let mut log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {3}).await?;
    let leader = router.get_raft_handle(&0)?;

    let metrics = leader.metrics().borrow_watched().clone();
    let leader_id = LeaderIdOf::<TypeConfig>::new_committed(metrics.current_term, metrics.current_leader.unwrap());
    let last_index = metrics.last_log_index.unwrap();
    let last_log_id = Some(LogIdOf::<TypeConfig>::new(leader_id, last_index));
    let earlier_log_id = Some(LogIdOf::<TypeConfig>::new(leader_id, last_index - 1));

    tracing::info!(log_index, "--- a change guarded by an earlier last log id is rejected");
    {
        let precondition = Precondition::LastLogId {
            last_log_id: earlier_log_id,
        };
        let err = leader.change_membership_if([0, 1, 2, 3], false, [precondition]).await.unwrap_err();

        let want = PreconditionFailed::LastLogIdMismatch {
            expected: earlier_log_id,
            actual: last_log_id,
        };
        assert_eq!(RaftError::APIError(ClientWriteError::PreconditionFailed(want)), err);
    }

    tracing::info!(log_index, "--- a change guarded by the current last log id is accepted");
    {
        let precondition = Precondition::LastLogId { last_log_id };
        leader.change_membership_if([0, 1, 2, 3], false, [precondition]).await?;
        log_index += 2;

        router.wait(&0, timeout()).applied_index(Some(log_index), "uniform config applied").await?;
    }

    Ok(())
}

/// A failing precondition on a follower must not mask the `ForwardToLeader` answer.
///
/// The core checks preconditions only after `ensure_leader_handler()`. A follower's replicated
/// state can trail the leader's, so checking first would report a concurrent change that never
/// happened instead of telling the caller where to retry.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn follower_answers_forward_to_leader() -> Result<()> {
    let config = Arc::new(
        Config {
            enable_heartbeat: false,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    let log_index = router.new_cluster(btreeset! {0,1,2}, btreeset! {3}).await?;
    let follower = router.get_raft_handle(&1)?;

    tracing::info!(
        log_index,
        "--- a precondition that cannot hold still yields ForwardToLeader"
    );
    {
        // An initialized node always has an effective membership log id, so `None` never holds.
        let precondition = Precondition::LastMembershipLogId {
            last_membership_log_id: None,
        };
        let err = follower.change_membership_if([0, 1, 2, 3], false, [precondition]).await.unwrap_err();

        let want = ClientWriteError::ForwardToLeader(ForwardToLeader::new(0, ()));
        assert_eq!(RaftError::APIError(want), err);
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(1000))
}
