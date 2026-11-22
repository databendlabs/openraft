use std::time::Duration;

use anyhow::Result;
use maplit::btreemap;
use maplit::btreeset;
use openraft::ServerState;
use openraft::SnapshotPolicy;
use openraft::alias::AsyncRuntimeOf;
use openraft::impls::ChangeMembers;
use openraft::impls::Membership;

use crate::fixtures::custom_membership::TestCluster;
use crate::fixtures::custom_membership::cluster_config;
use crate::fixtures::ut_harness;

openraft::declare_raft_types!(
    TypeConfig:
        D = String,
        R = (),
        Node = String,
        AsyncRuntime = AsyncRuntimeOf<openraft_memstore::TypeConfig>,
);

/// A membership change that moves learner 2 to a new address makes leader 1 create new clients
/// for 2 from that address, so replication reaches 2 after its old address closes. Learner 3,
/// whose address stays, keeps its clients. Removing 2 and adding it back at a third address
/// replaces its clients again.
#[tracing::instrument]
#[test_harness::test(harness = ut_harness)]
async fn set_nodes_replaces_the_clients_of_a_changed_node() -> Result<()> {
    let nodes = btreemap! {1 => s("n1"), 2 => s("old"), 3 => s("n3")};
    let membership = Membership::new(vec![btreeset! {1}], nodes)?;
    let cluster =
        TestCluster::<TypeConfig>::start(cluster_config(SnapshotPolicy::Never), [1, 2, 3], membership).await?;

    tracing::info!("--- node 1 leads, and learners 2 and 3 apply its blank entry");
    let mut clients = {
        cluster.raft(1).trigger().elect(false).await?;
        cluster.raft(1).wait(timeout()).state(ServerState::Leader, "node 1 becomes leader").await?;
        for node_id in [2, 3] {
            cluster
                .raft(node_id)
                .wait(timeout())
                .applied_index(Some(1), "learner applies the blank entry")
                .await?;
        }

        let want = vec![
            (1, 2, "new_heartbeat_client", s("old")),
            (1, 3, "new_heartbeat_client", s("n3")),
            (1, 2, "new_client", s("old")),
            (1, 3, "new_client", s("n3")),
        ];
        assert_eq!(want, cluster.clients());
        want
    };

    tracing::info!("--- the old address of node 2 closes, and node 1 moves node 2 to a new address");
    let set_nodes_index = {
        cluster.close_endpoint(2, s("old"));
        let change = ChangeMembers::SetNodes(btreemap! {2 => s("new")});
        let changed = cluster.raft(1).change_membership(change, true).await?;
        changed.log_id.index
    };

    tracing::info!(
        set_nodes_index,
        "--- node 2 catches up through clients created from its new address"
    );
    {
        let node_2 = cluster.raft(2);
        node_2.wait(timeout()).applied_index(Some(set_nodes_index), "node 2 applies the change").await?;

        clients.push((1, 2, "new_heartbeat_client", s("new")));
        clients.push((1, 2, "new_client", s("new")));
        assert_eq!(clients, cluster.clients(), "only node 2 gets new clients");
    }

    tracing::info!(set_nodes_index, "--- a later write reaches both learners");
    {
        let response = cluster.raft(1).client_write(s("after the move")).await?;
        let index = response.log_id.index;
        for node_id in [2, 3] {
            cluster
                .raft(node_id)
                .wait(timeout())
                .applied_index(Some(index), "learner applies the write")
                .await?;
        }
        assert_eq!(clients, cluster.clients(), "the write creates no client");
    }

    tracing::info!("--- node 1 removes node 2, and adds it back at a third address");
    {
        cluster.close_endpoint(2, s("new"));
        let remove = ChangeMembers::RemoveNodes(btreeset! {2});
        cluster.raft(1).change_membership(remove, false).await?;
        assert_eq!(clients, cluster.clients(), "removing node 2 creates no client");

        let added = cluster.raft(1).add_learner(2, s("newer"), true).await?;
        let index = added.log_id.index;
        cluster.raft(2).wait(timeout()).applied_index(Some(index), "node 2 applies its addition").await?;

        clients.push((1, 2, "new_heartbeat_client", s("newer")));
        clients.push((1, 2, "new_client", s("newer")));
        assert_eq!(clients, cluster.clients());
    }

    Ok(())
}

fn s(x: &str) -> String {
    x.to_string()
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2_000))
}
