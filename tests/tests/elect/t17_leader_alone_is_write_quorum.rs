use std::fmt;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::RaftTypeConfig;
use openraft::ReadPolicy;
use openraft::ServerState;
use openraft::alias::AsyncRuntimeOf;
use openraft::alias::EntryOf;
use openraft::entry::RaftEntry;
use openraft::testing::log_id;
use openraft::type_config::TypeConfigExt;

use crate::fixtures::custom_membership::MinimalQuorums;
use crate::fixtures::custom_membership::MinimalQuorumsMembership;
use crate::fixtures::custom_membership::TestCluster;
use crate::fixtures::custom_membership::TestConfig;
use crate::fixtures::ut_harness;

/// Each of nodes 1, 2 and 3 alone is a write quorum. The only read quorum holds all three nodes,
/// so every read quorum meets every write quorum.
fn every_node_is_a_write_quorum() -> MinimalQuorumsMembership {
    MinimalQuorumsMembership {
        read: MinimalQuorums(vec![btreeset! {1, 2, 3}]),
        write: MinimalQuorums(vec![btreeset! {1}, btreeset! {2}, btreeset! {3}]),
        ..Default::default()
    }
}

openraft::declare_raft_types!(
    StdConfig:
        D = String,
        R = (),
        Node = (),
        LeaderId = openraft::impls::leader_id_std::LeaderId<u64, u64>,
        Membership = MinimalQuorumsMembership,
        AsyncRuntime = AsyncRuntimeOf<openraft_memstore::TypeConfig>,
);

openraft::declare_raft_types!(
    AdvConfig:
        D = String,
        R = (),
        Node = (),
        Membership = MinimalQuorumsMembership,
        AsyncRuntime = AsyncRuntimeOf<openraft_memstore::TypeConfig>,
);

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn leader_alone_keeps_lease_with_standard_leader_ids() -> Result<()> {
    leader_alone_keeps_lease::<StdConfig>().await
}

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn leader_alone_keeps_lease_with_advanced_leader_ids() -> Result<()> {
    leader_alone_keeps_lease::<AdvConfig>().await
}

/// A leader that alone forms a write quorum keeps serving reads and writes after its followers
/// become unreachable and the lease from their last acknowledgments runs out.
///
/// No other candidate can win: every read quorum contains the leader, and the leader refuses to
/// vote while it leads.
async fn leader_alone_keeps_lease<C>() -> Result<()>
where
    C: TestConfig + RaftTypeConfig<D = String, Membership = MinimalQuorumsMembership>,
    EntryOf<C>: Clone + PartialEq + fmt::Debug,
{
    let cluster = TestCluster::<C>::start(btreeset! {1, 2, 3}, every_node_is_a_write_quorum()).await?;

    tracing::info!("--- node 1 wins the votes of all three nodes and commits its blank entry");
    {
        cluster.raft(1).trigger().elect(false).await?;
        cluster.raft(1).wait(timeout()).state(ServerState::Leader, "node 1 becomes leader").await?;
        cluster.raft(1).wait(timeout()).applied_index(Some(1), "node 1 commits its blank entry").await?;
    }

    tracing::info!("--- cut node 1 from both followers and wait until the lease runs out");
    {
        cluster.cut_links(btreeset! {(1, 2), (1, 3)});

        // `leader_lease` equals `election_timeout_max`, see `EngineConfig::new`.
        let leader_lease = Duration::from_millis(cluster.raft(1).config().election_timeout_max);
        C::sleep(leader_lease + Duration::from_millis(200)).await;
    }

    tracing::info!("--- node 1 alone confirms a linearizable read");
    {
        let read_log_id = cluster.raft(1).ensure_linearizable(ReadPolicy::ReadIndex).await?;
        assert_eq!(&log_id::<C>(1, 1, 1), read_log_id.log_id());
    }

    tracing::info!("--- node 1 alone commits and applies a write");
    {
        let response = cluster.raft(1).client_write("alone".to_string()).await?;
        assert_eq!(log_id::<C>(1, 1, 2), response.log_id);

        let want = vec![
            EntryOf::<C>::new_membership(log_id::<C>(0, 0, 0), every_node_is_a_write_quorum()),
            EntryOf::<C>::new_blank(log_id::<C>(1, 1, 1)),
            EntryOf::<C>::new_normal(log_id::<C>(1, 1, 2), "alone".to_string()),
        ];
        let applied = cluster.applied(1);
        assert_eq!(want, applied);
    }

    Ok(())
}

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn transferring_leader_alone_loses_lease_with_standard_leader_ids() -> Result<()> {
    transferring_leader_alone_loses_lease::<StdConfig>().await
}

#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn transferring_leader_alone_loses_lease_with_advanced_leader_ids() -> Result<()> {
    transferring_leader_alone_loses_lease::<AdvConfig>().await
}

/// A leader that alone forms a write quorum gives up its lease while it transfers leadership.
///
/// The transfer target, node 4, is not running, so the transfer never completes. The leader sends
/// no heartbeat during a transfer, so its lease runs out, and it votes for a candidate whose log
/// is as new as its own.
async fn transferring_leader_alone_loses_lease<C>() -> Result<()>
where
    C: TestConfig + RaftTypeConfig<D = String, Membership = MinimalQuorumsMembership>,
    EntryOf<C>: Clone + PartialEq + fmt::Debug,
{
    let cluster = TestCluster::<C>::start(btreeset! {1, 2, 3}, every_node_is_a_write_quorum()).await?;

    tracing::info!("--- node 1 wins term 1, and every node stores its blank entry");
    {
        cluster.raft(1).trigger().elect(false).await?;
        cluster.raft(1).wait(timeout()).state(ServerState::Leader, "node 1 becomes leader").await?;
        for node_id in 1..=3 {
            cluster.raft(node_id).wait(timeout()).log_index(Some(1), "store the blank entry").await?;
        }
    }

    tracing::info!("--- node 1 transfers leadership to node 4, which is not running; wait until the lease runs out");
    {
        cluster.raft(1).trigger().transfer_leader(4).await?;

        // `leader_lease` equals `election_timeout_max`, see `EngineConfig::new`.
        let leader_lease = Duration::from_millis(cluster.raft(1).config().election_timeout_max);
        C::sleep(leader_lease + Duration::from_millis(200)).await;
    }

    tracing::info!("--- node 2 campaigns in term 2, and node 1 votes for it and steps down");
    {
        cluster.raft(2).trigger().elect(false).await?;
        cluster.raft(2).wait(timeout()).state(ServerState::Leader, "node 2 becomes leader").await?;
        cluster.raft(1).wait(timeout()).state(ServerState::Follower, "node 1 steps down").await?;
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2_000))
}
