use std::fmt;
use std::num::NonZeroU64;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::RaftTypeConfig;
use openraft::ReadPolicy;
use openraft::ServerState;
use openraft::Vote;
use openraft::alias::AsyncRuntimeOf;
use openraft::alias::EntryOf;
use openraft::alias::VoteOf;
use openraft::async_runtime::WatchReceiver;
use openraft::entry::RaftEntry;
use openraft::errors::LinearizableReadError;
use openraft::errors::QuorumNotEnough;
use openraft::errors::RaftError;
use openraft::membership::Joint;
use openraft::membership::NodeRole;
use openraft::membership::RaftMembership;
use openraft::membership::ThresholdChange;
use openraft::membership::ThresholdMembership;
use openraft::membership::ThresholdQuorumSet;
use openraft::testing::log_id;
use openraft::type_config::TypeConfigExt;

use crate::fixtures::TestingVoteExt;
use crate::fixtures::custom_membership::TestCluster;
use crate::fixtures::custom_membership::TestConfig;
use crate::fixtures::ut_harness;

openraft::declare_raft_types!(
    StdConfig:
        D = String,
        R = (),
        Node = (),
        LeaderId = openraft::impls::leader_id_std::LeaderId<u64, u64>,
        Membership = ThresholdMembership<u64, ()>,
        AsyncRuntime = AsyncRuntimeOf<openraft_memstore::TypeConfig>,
);

openraft::declare_raft_types!(
    AdvConfig:
        D = String,
        R = (),
        Node = (),
        Membership = ThresholdMembership<u64, ()>,
        AsyncRuntime = AsyncRuntimeOf<openraft_memstore::TypeConfig>,
);

/// The voters 1 to 5 with one config per write size, the oldest config first.
/// Two configs form a joint membership.
fn five_voters(sizes: &[u64]) -> ThresholdMembership<u64, ()> {
    let voters = Arc::new(btreeset! {1, 2, 3, 4, 5});
    let mut read_rules = vec![];
    let mut write_rules = vec![];
    for &write_size in sizes {
        let read_size = voters.len() as u64 - write_size + 1;
        let read_size = NonZeroU64::new(read_size).unwrap();
        let write_size = NonZeroU64::new(write_size).unwrap();
        read_rules.push(ThresholdQuorumSet::new(read_size, voters.clone()));
        write_rules.push(ThresholdQuorumSet::new(write_size, voters.clone()));
    }

    let read = Joint::new(read_rules);
    let write = Joint::new(write_rules);
    let voter_role = NodeRole::Campaigner | NodeRole::ReceiveLogs;
    let nodes = voters.iter().map(|id| (*id, (), voter_role));
    RaftMembership::new(read, write, nodes)
}

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn small_write_quorum_commit_survives_leader_change_with_standard_leader_ids() -> Result<()> {
    small_write_quorum_commit_survives_leader_change::<StdConfig>().await
}

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn small_write_quorum_commit_survives_leader_change_with_advanced_leader_ids() -> Result<()> {
    small_write_quorum_commit_survives_leader_change::<AdvConfig>().await
}

/// With 5 voters and (4,2), an entry commits once 2 voters store it, and an election needs 4
/// votes. Any 4 voters include one of the 2 that store a committed entry, and that voter refuses a
/// candidate whose log lacks the entry. So every new leader holds the entry.
async fn small_write_quorum_commit_survives_leader_change<C>() -> Result<()>
where
    C: TestConfig + RaftTypeConfig<D = String, Membership = ThresholdMembership<u64, ()>>,
    EntryOf<C>: Clone + PartialEq + fmt::Debug,
{
    let cluster = TestCluster::<C>::start(btreeset! {1, 2, 3, 4, 5}, five_voters(&[2])).await?;

    tracing::info!("--- node 1 wins term 1 and commits its blank entry to start its lease; all nodes store the entry");
    {
        cluster.raft(1).trigger().elect(false).await?;
        cluster.raft(1).wait(timeout()).state(ServerState::Leader, "node 1 becomes leader").await?;
        cluster.raft(1).wait(timeout()).applied_index(Some(1), "node 1 commits its blank entry").await?;
        for node_id in 1..=5 {
            cluster.raft(node_id).wait(timeout()).log_index(Some(1), "store the blank entry").await?;
        }
    }

    tracing::info!("--- cut node 1 from nodes 3, 4 and 5; x commits once nodes 1 and 2 store it");
    {
        cluster.cut_links(btreeset! {(1, 3), (1, 4), (1, 5)});

        let response = cluster.raft(1).client_write("x".to_string()).await?;
        assert_eq!(log_id::<C>(1, 1, 2), response.log_id);

        for node_id in 3..=5 {
            let raft = cluster.raft(node_id);
            let metrics = raft.metrics().borrow_watched().clone();
            assert_eq!(Some(1), metrics.last_log_index, "node {node_id} lacks x");
        }
    }

    tracing::info!("--- cut node 1 from node 2 too, and wait until the lease of node 1 runs out");
    {
        cluster.cut_links(btreeset! {(1, 2), (1, 3), (1, 4), (1, 5)});

        // `leader_lease` equals `election_timeout_max`, see `EngineConfig::new`.
        let leader_lease = Duration::from_millis(cluster.raft(1).config().election_timeout_max);
        C::sleep(leader_lease + Duration::from_millis(200)).await;
    }

    tracing::info!("--- node 3 campaigns in term 2; nodes 4 and 5 vote for it, node 2 refuses because node 3 lacks x");
    {
        cluster.raft(3).trigger().elect(false).await?;

        let vote = VoteOf::<C>::from_term_node_id(2, 3);
        cluster.raft(4).wait(timeout()).vote(vote.clone(), "node 4 votes for node 3").await?;
        cluster.raft(5).wait(timeout()).vote(vote, "node 5 votes for node 3").await?;

        C::sleep(Duration::from_millis(500)).await;

        let raft = cluster.raft(3);
        let metrics = raft.metrics().borrow_watched().clone();
        assert_eq!(ServerState::Candidate, metrics.state, "3 votes are no read quorum");
    }

    tracing::info!("--- node 2 campaigns twice: nodes 3, 4 and 5 voted for node 3 in term 2, so node 2 wins term 3");
    {
        cluster.raft(2).trigger().elect(false).await?;
        cluster.raft(2).trigger().elect(false).await?;
        cluster.raft(2).wait(timeout()).state(ServerState::Leader, "node 2 becomes leader").await?;
    }

    tracing::info!("--- nodes 2 to 5 apply x and then the blank entry of node 2");
    {
        let want = vec![
            EntryOf::<C>::new_membership(log_id::<C>(0, 0, 0), five_voters(&[2])),
            EntryOf::<C>::new_blank(log_id::<C>(1, 1, 1)),
            EntryOf::<C>::new_normal(log_id::<C>(1, 1, 2), "x".to_string()),
            EntryOf::<C>::new_blank(log_id::<C>(3, 2, 3)),
        ];
        for node_id in 2..=5 {
            cluster
                .raft(node_id)
                .wait(timeout())
                .applied_index(Some(3), "apply the blank entry of node 2")
                .await?;
            let applied = cluster.applied(node_id);
            assert_eq!(want, applied, "node {node_id}");
        }
    }

    Ok(())
}

/// A lease blocks an election by a read quorum without the leader.
///
/// With 4 voters and (2,3), the read quorum {3,4} shares no voter with the read quorum {1,2}.
/// Suppose node 1 renewed its lease through {1,2} alone. Then nodes 3 and 4 could elect a second
/// leader that commits through {2,3,4}, while node 1 still served lease reads that miss those
/// writes. A lease counts a write quorum instead, here {1,2,3}, and every read quorum without node
/// 1 contains node 2 or node 3.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn lease_blocks_election_by_read_quorum_without_leader() -> Result<()> {
    let voters = btreeset! {1, 2, 3, 4};
    let membership = ThresholdMembership::<u64, ()>::new(voters.clone(), 3, voters.clone())?;
    let cluster = TestCluster::<AdvConfig>::start(voters, membership).await?;

    tracing::info!("--- node 1 wins term 1, and every node stores its blank entry");
    {
        cluster.raft(1).trigger().elect(false).await?;
        cluster.raft(1).wait(timeout()).state(ServerState::Leader, "node 1 becomes leader").await?;
        for node_id in 1..=4 {
            cluster.raft(node_id).wait(timeout()).log_index(Some(1), "store the blank entry").await?;
        }
    }

    tracing::info!("--- cut the link 1-4; a ReadIndex read renews the lease of node 1 through nodes 2 and 3");
    {
        cluster.cut_links(btreeset! {(1, 4)});
        cluster.raft(1).ensure_linearizable(ReadPolicy::ReadIndex).await?;
    }

    tracing::info!("--- node 4 campaigns in term 2 with a log as new as theirs; nodes 2 and 3 refuse within the lease");
    {
        cluster.raft(4).trigger().elect(false).await?;
        AdvConfig::sleep(Duration::from_millis(100)).await;

        let raft = cluster.raft(4);
        let metrics = raft.metrics().borrow_watched().clone();
        assert_eq!(ServerState::Candidate, metrics.state, "node 4 gets no vote");

        for node_id in [2, 3] {
            let raft = cluster.raft(node_id);
            let metrics = raft.metrics().borrow_watched().clone();
            assert_eq!(
                Vote::new_committed(1, 1),
                metrics.vote,
                "node {node_id} keeps its vote for node 1"
            );
        }
    }

    tracing::info!("--- node 1 is still leader and serves a lease read");
    {
        let raft = cluster.raft(1);
        let metrics = raft.metrics().borrow_watched().clone();
        assert_eq!(ServerState::Leader, metrics.state);

        let read_log_id = raft.ensure_linearizable(ReadPolicy::LeaseRead).await?;
        assert_eq!(&log_id::<AdvConfig>(1, 1, 1), read_log_id.log_id());
    }

    Ok(())
}

/// Switching 5 voters from (4,2) to (2,4) passes through a joint config, because a read quorum of
/// (2,4) can miss a write quorum of (4,2). After the switch, an entry commits only once 4 voters
/// store it.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn sizes_only_change_passes_through_joint_config() -> Result<()> {
    let cluster = TestCluster::<AdvConfig>::start(btreeset! {1, 2, 3, 4, 5}, five_voters(&[2])).await?;

    tracing::info!("--- node 1 wins term 1 and commits its blank entry");
    {
        cluster.raft(1).trigger().elect(false).await?;
        cluster.raft(1).wait(timeout()).applied_index(Some(1), "node 1 commits its blank entry").await?;
    }

    tracing::info!("--- switch to (2,4): node 1 commits the joint config at index 2, then (2,4) at index 3");
    {
        let change = ThresholdChange::write_size(4);
        let response = cluster.raft(1).change_membership(change, false).await?;
        assert_eq!(log_id::<AdvConfig>(1, 1, 3), response.log_id);
    }

    tracing::info!("--- cut node 1 from nodes 4 and 5; nodes 1, 2 and 3 store y at index 4, which does not commit");
    {
        cluster.cut_links(btreeset! {(1, 4), (1, 5)});
        cluster.raft(1).client_write_ff("y".to_string(), None).await?;

        for node_id in 1..=3 {
            cluster.raft(node_id).wait(timeout()).log_index(Some(4), "store y").await?;
        }
        AdvConfig::sleep(Duration::from_millis(500)).await;

        let raft = cluster.raft(1);
        let metrics = raft.metrics().borrow_watched().clone();
        assert_eq!(
            Some(log_id::<AdvConfig>(1, 1, 3)),
            metrics.cluster_committed,
            "3 voters are no write quorum"
        );
    }

    tracing::info!("--- restore the link 1-4; nodes 1 to 4 form a write quorum and commit y");
    {
        cluster.cut_links(btreeset! {(1, 5)});
        cluster.raft(1).wait(timeout()).applied_index(Some(4), "node 1 applies y").await?;

        let want = vec![
            EntryOf::<AdvConfig>::new_membership(log_id::<AdvConfig>(0, 0, 0), five_voters(&[2])),
            EntryOf::<AdvConfig>::new_blank(log_id::<AdvConfig>(1, 1, 1)),
            EntryOf::<AdvConfig>::new_membership(log_id::<AdvConfig>(1, 1, 2), five_voters(&[2, 4])),
            EntryOf::<AdvConfig>::new_membership(log_id::<AdvConfig>(1, 1, 3), five_voters(&[4])),
            EntryOf::<AdvConfig>::new_normal(log_id::<AdvConfig>(1, 1, 4), "y".to_string()),
        ];
        let applied = cluster.applied(1);
        assert_eq!(want, applied);
    }

    Ok(())
}

/// With 5 voters and (4,2), a write quorum of 2 voters confirms a linearizable read, although an
/// election needs 4 votes.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn linearizable_read_needs_only_write_quorum() -> Result<()> {
    let membership = five_voters(&[2]);
    let cluster = TestCluster::<AdvConfig>::start(btreeset! {1, 2, 3, 4, 5}, membership.clone()).await?;

    tracing::info!("--- node 1 wins term 1 and commits its blank entry");
    {
        cluster.raft(1).trigger().elect(false).await?;
        cluster.raft(1).wait(timeout()).applied_index(Some(1), "node 1 commits its blank entry").await?;
    }

    tracing::info!("--- cut node 1 from nodes 3, 4 and 5; nodes 1 and 2 confirm reads");
    {
        cluster.cut_links(btreeset! {(1, 3), (1, 4), (1, 5)});

        let read_log_id = cluster.raft(1).ensure_linearizable(ReadPolicy::ReadIndex).await?;
        assert_eq!(&log_id::<AdvConfig>(1, 1, 1), read_log_id.log_id());

        let read_log_id = cluster.raft(1).ensure_linearizable(ReadPolicy::LeaseRead).await?;
        assert_eq!(&log_id::<AdvConfig>(1, 1, 1), read_log_id.log_id());
    }

    tracing::info!("--- cut node 1 from node 2 too; node 1 alone confirms no read");
    {
        cluster.cut_links(btreeset! {(1, 2), (1, 3), (1, 4), (1, 5)});

        let result = cluster.raft(1).ensure_linearizable(ReadPolicy::ReadIndex).await;
        let Err(RaftError::APIError(LinearizableReadError::QuorumNotEnough(quorum_not_enough))) = result else {
            panic!("expected QuorumNotEnough, got: {result:?}");
        };
        let want = QuorumNotEnough {
            cluster: format!("{:?}", membership.write_quorum_set()),
            got: btreeset! {1},
        };
        assert_eq!(want, quorum_not_enough);
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2_000))
}
