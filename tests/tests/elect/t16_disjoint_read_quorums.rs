use std::collections::BTreeSet;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::ServerState;
use openraft::Vote;
use openraft::alias::AsyncRuntimeOf;
use openraft::alias::EntryOf;
use openraft::async_runtime::WatchReceiver;
use openraft::entry::RaftEntry;
use openraft::errors::ClientWriteError;
use openraft::errors::ForwardReason;
use openraft::errors::ForwardToLeader;
use openraft::errors::RaftError;
use openraft::testing::log_id;
use openraft::type_config::TypeConfigExt;

use crate::fixtures::custom_membership::MinimalQuorums;
use crate::fixtures::custom_membership::MinimalQuorumsMembership;
use crate::fixtures::custom_membership::TestCluster;
use crate::fixtures::ut_harness;

/// The grid of `openraft/src/docs/protocol/election_quorum.md`: each row is a read quorum and each
/// column is a write quorum. The rows share no voter.
///
/// ```text
///             write {1,3}   write {2,4}
/// read {1,2}       1             2
/// read {3,4}       3             4
/// ```
fn grid() -> MinimalQuorumsMembership {
    MinimalQuorumsMembership {
        read: MinimalQuorums(vec![btreeset! {1, 2}, btreeset! {3, 4}]),
        write: MinimalQuorums(vec![btreeset! {1, 3}, btreeset! {2, 4}]),
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

/// The links between the rows `{1,2}` and `{3,4}` of the grid.
fn links_between_rows() -> BTreeSet<(u64, u64)> {
    btreeset! {(1, 3), (1, 4), (2, 3), (2, 4)}
}

/// With standard leader IDs, a row of the grid is a read quorum but not a write quorum, so it
/// cannot elect a leader.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn standard_leader_ids_need_a_read_and_a_write_quorum() -> Result<()> {
    let cluster = TestCluster::<StdConfig>::start(btreeset! {1, 2, 3, 4}, grid()).await?;

    tracing::info!("--- cut the rows {{1,2}} and {{3,4}} apart, then campaign at nodes 1 and 4 in term 1");
    {
        cluster.cut_links(links_between_rows());
        cluster.raft(1).trigger().elect(false).await?;
        cluster.raft(4).trigger().elect(false).await?;
    }

    tracing::info!("--- node 2 votes for node 1 and node 3 for node 4, yet neither candidate becomes leader");
    {
        cluster.raft(2).wait(timeout()).vote(Vote::new(1, 1), "node 2 votes for node 1").await?;
        cluster.raft(3).wait(timeout()).vote(Vote::new(1, 4), "node 3 votes for node 4").await?;

        StdConfig::sleep(Duration::from_millis(500)).await;

        for node_id in [1, 4] {
            let raft = cluster.raft(node_id);
            let metrics = raft.metrics().borrow_watched().clone();
            assert_eq!(ServerState::Candidate, metrics.state, "node {node_id}");
        }
    }

    tracing::info!("--- restore the links; node 4 wins term 2 with votes from both rows and commits a write");
    {
        cluster.cut_links(btreeset! {});
        cluster.raft(4).trigger().elect(false).await?;
        cluster.raft(4).wait(timeout()).applied_index(Some(1), "node 4 commits its blank entry").await?;

        let response = cluster.raft(4).client_write("from-4".to_string()).await?;
        assert_eq!(log_id::<StdConfig>(2, 4, 2), response.log_id);
    }

    tracing::info!("--- every node applies the same entries");
    {
        let want = vec![
            EntryOf::<StdConfig>::new_membership(log_id::<StdConfig>(0, 0, 0), grid()),
            EntryOf::<StdConfig>::new_blank(log_id::<StdConfig>(2, 4, 1)),
            EntryOf::<StdConfig>::new_normal(log_id::<StdConfig>(2, 4, 2), "from-4".to_string()),
        ];
        for node_id in 1..=4 {
            cluster.raft(node_id).wait(timeout()).applied_index(Some(2), "apply the write").await?;
            let applied = cluster.applied(node_id);
            assert_eq!(want, applied, "node {node_id}");
        }
    }

    Ok(())
}

/// With advanced leader IDs, each row of the grid elects a leader in the same term. Every write
/// quorum spans both rows, so neither leader commits or accepts a write while the rows are cut
/// apart. After the links return, only the entries of the greater leader ID commit.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn advanced_leader_ids_elect_by_read_quorum_and_commit_by_write_quorum() -> Result<()> {
    let cluster = TestCluster::<AdvConfig>::start(btreeset! {1, 2, 3, 4}, grid()).await?;

    tracing::info!("--- cut the rows {{1,2}} and {{3,4}} apart; nodes 1 and 4 both win term 1");
    {
        cluster.cut_links(links_between_rows());
        cluster.raft(1).trigger().elect(false).await?;
        cluster.raft(4).trigger().elect(false).await?;

        cluster.raft(1).wait(timeout()).state(ServerState::Leader, "row {1,2} elects node 1").await?;
        cluster.raft(4).wait(timeout()).state(ServerState::Leader, "row {3,4} elects node 4").await?;
    }

    tracing::info!("--- each row stores the blank entry of its own leader at index 1");
    {
        cluster.raft(2).wait(timeout()).log_index(Some(1), "node 2 stores node 1's blank entry").await?;
        cluster.raft(3).wait(timeout()).log_index(Some(1), "node 3 stores node 4's blank entry").await?;
    }

    tracing::info!("--- no write quorum has acknowledged either leader, so both reject writes");
    {
        for node_id in [1, 4] {
            let data = format!("from-{node_id}");
            let result = cluster.raft(node_id).client_write(data).await;
            let err = result.unwrap_err();
            let forward = ForwardToLeader {
                leader_id: None,
                leader_node: None,
                reason: ForwardReason::LeaseExpired,
            };
            let want = RaftError::APIError(ClientWriteError::ForwardToLeader(forward));
            assert_eq!(want, err, "node {node_id}");
        }
    }

    tracing::info!("--- restore the links; node 4, the greater leader ID, commits its blank entry and a write");
    {
        cluster.cut_links(btreeset! {});
        cluster.raft(4).wait(timeout()).applied_index(Some(1), "node 4 commits its blank entry").await?;

        let response = cluster.raft(4).client_write("from-4".to_string()).await?;
        assert_eq!(log_id::<AdvConfig>(1, 4, 2), response.log_id);
    }

    tracing::info!("--- every node applies the entries of node 4, and none of node 1");
    {
        let want = vec![
            EntryOf::<AdvConfig>::new_membership(log_id::<AdvConfig>(0, 0, 0), grid()),
            EntryOf::<AdvConfig>::new_blank(log_id::<AdvConfig>(1, 4, 1)),
            EntryOf::<AdvConfig>::new_normal(log_id::<AdvConfig>(1, 4, 2), "from-4".to_string()),
        ];
        for node_id in 1..=4 {
            cluster.raft(node_id).wait(timeout()).applied_index(Some(2), "apply the write of node 4").await?;
            let applied = cluster.applied(node_id);
            assert_eq!(want, applied, "node {node_id}");
        }
    }

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2_000))
}
