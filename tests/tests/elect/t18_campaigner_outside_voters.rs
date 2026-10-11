use std::time::Duration;

use anyhow::Result;
use maplit::btreemap;
use maplit::btreeset;
use openraft::ServerState;
use openraft::Vote;
use openraft::alias::AsyncRuntimeOf;
use openraft::alias::EntryOf;
use openraft::async_runtime::WatchReceiver;
use openraft::entry::RaftEntry;
use openraft::membership::NodeRole;
use openraft::testing::log_id;
use openraft::type_config::TypeConfigExt;

use crate::fixtures::custom_membership::MinimalQuorums;
use crate::fixtures::custom_membership::MinimalQuorumsMembership;
use crate::fixtures::custom_membership::TestCluster;
use crate::fixtures::ut_harness;

/// Node 1 alone is the read and the write quorum. Node 2 is in neither quorum set, but it may
/// campaign and it receives logs.
fn voter_1_and_campaigner_2() -> MinimalQuorumsMembership {
    MinimalQuorumsMembership {
        read: MinimalQuorums(vec![btreeset! {1}]),
        write: MinimalQuorums(vec![btreeset! {1}]),
        roles: btreemap! {2 => NodeRole::Campaigner | NodeRole::ReceiveLogs},
    }
}

openraft::declare_raft_types!(
    TypeConfig:
        D = String,
        R = (),
        Node = (),
        Membership = MinimalQuorumsMembership,
        AsyncRuntime = AsyncRuntimeOf<openraft_memstore::TypeConfig>,
);

/// A campaigner outside the voter sets waits for its election timeout and runs Pre-Vote.
///
/// Only the sole voter may skip both, because its own vote wins the election. Node 2 needs the
/// vote of node 1, which node 1 refuses while it leads, so a Pre-Vote of node 2 fails and its term
/// stays 1.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn campaigner_outside_voters_runs_pre_vote() -> Result<()> {
    let cluster = TestCluster::<TypeConfig>::start(btreeset! {1, 2}, voter_1_and_campaigner_2()).await?;

    tracing::info!("--- node 1 wins term 1, and node 2 stores its blank entry");
    {
        cluster.raft(1).trigger().elect(false).await?;
        cluster.raft(1).wait(timeout()).state(ServerState::Leader, "node 1 becomes leader").await?;
        cluster.raft(2).wait(timeout()).log_index(Some(1), "node 2 stores the blank entry").await?;
    }

    tracing::info!(
        "--- tick node 2 with Pre-Vote for longer than any election timeout; node 1 refuses the Pre-Vote, so the term stays 1"
    );
    {
        let n2 = cluster.raft(2);
        n2.runtime_config().pre_vote(true);
        n2.runtime_config().tick(true);

        let election_timeout_max = Duration::from_millis(n2.config().election_timeout_max);
        TypeConfig::sleep(election_timeout_max * 2).await;

        let metrics = n2.metrics().borrow_watched().clone();
        assert_eq!(Vote::new_committed(1, 1), metrics.vote);
    }

    Ok(())
}

/// A campaigner outside the voter sets wins an election with the vote of node 1, the only voter.
///
/// Node 2 campaigns before node 1 has led, because node 1 refuses every vote while it leads. As
/// leader, node 2 commits each entry once node 1, the only write voter, stores it.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn campaigner_outside_voters_wins_election() -> Result<()> {
    let cluster = TestCluster::<TypeConfig>::start(btreeset! {1, 2}, voter_1_and_campaigner_2()).await?;

    tracing::info!("--- node 2 campaigns in term 1, wins the vote of node 1, and commits its blank entry");
    {
        cluster.raft(2).trigger().elect(false).await?;
        cluster.raft(2).wait(timeout()).state(ServerState::Leader, "node 2 becomes leader").await?;
        cluster.raft(2).wait(timeout()).applied_index(Some(1), "node 2 commits its blank entry").await?;
    }

    tracing::info!("--- node 2 commits a write, and both nodes apply the entries of node 2");
    {
        let response = cluster.raft(2).client_write("from-2".to_string()).await?;
        assert_eq!(log_id::<TypeConfig>(1, 2, 2), response.log_id);

        let want = vec![
            EntryOf::<TypeConfig>::new_membership(log_id::<TypeConfig>(0, 0, 0), voter_1_and_campaigner_2()),
            EntryOf::<TypeConfig>::new_blank(log_id::<TypeConfig>(1, 2, 1)),
            EntryOf::<TypeConfig>::new_normal(log_id::<TypeConfig>(1, 2, 2), "from-2".to_string()),
        ];
        for node_id in [1, 2] {
            cluster.raft(node_id).wait(timeout()).applied_index(Some(2), "apply the write of node 2").await?;
            let applied = cluster.applied(node_id);
            assert_eq!(want, applied, "node {node_id}");
        }
    }

    Ok(())
}

/// A transfer delivers its request to a campaigner outside both quorum sets and lets it commit.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn leadership_transfers_to_campaigner_outside_voters() -> Result<()> {
    let membership = voter_1_and_campaigner_2();
    let cluster = TestCluster::<TypeConfig>::start(btreeset! {1, 2}, membership.clone()).await?;

    tracing::info!("--- elect node 1 and catch up the non-voter transfer target, node 2");
    {
        cluster.raft(1).trigger().elect(false).await?;
        for node_id in [1, 2] {
            cluster
                .raft(node_id)
                .wait(timeout())
                .applied_index(Some(1), "apply the blank entry of node 1")
                .await?;
        }
    }

    tracing::info!("--- transfer leadership to node 2 with ticking disabled on both nodes");
    {
        cluster.raft(1).trigger().transfer_leader(2).await?;
        cluster
            .raft(2)
            .wait(timeout())
            .state(ServerState::Leader, "the non-voter target becomes leader")
            .await?;
        cluster.raft(2).wait(timeout()).applied_index(Some(2), "node 2 commits its blank entry").await?;
    }

    tracing::info!("--- keep heartbeats running and ensure the sole voter honors the new leader's lease");
    {
        let leader = cluster.raft(2);
        leader.runtime_config().tick(true);
        let voter = cluster.raft(1);
        voter.runtime_config().tick(true);
        let election_timeout_max = Duration::from_millis(voter.config().election_timeout_max);
        TypeConfig::sleep(election_timeout_max * 2).await;

        let metrics = voter.metrics().borrow_watched().clone();
        let expected_vote = Vote::new_committed(2, 2);
        assert_eq!(expected_vote, metrics.vote);
        assert_eq!(ServerState::Follower, metrics.state);
    }

    tracing::info!("--- the new leader commits a write through node 1, the only write voter");
    {
        let response = cluster.raft(2).client_write("after-transfer".to_string()).await?;
        let expected_log_id = log_id::<TypeConfig>(2, 2, 3);
        assert_eq!(expected_log_id, response.log_id);

        let want = vec![
            EntryOf::<TypeConfig>::new_membership(log_id::<TypeConfig>(0, 0, 0), membership),
            EntryOf::<TypeConfig>::new_blank(log_id::<TypeConfig>(1, 1, 1)),
            EntryOf::<TypeConfig>::new_blank(log_id::<TypeConfig>(2, 2, 2)),
            EntryOf::<TypeConfig>::new_normal(expected_log_id, "after-transfer".to_string()),
        ];
        for node_id in [1, 2] {
            cluster.raft(node_id).wait(timeout()).applied_index(Some(3), "apply the write of node 2").await?;
            let applied = cluster.applied(node_id);
            assert_eq!(want, applied, "node {node_id}");
        }
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2_000))
}
