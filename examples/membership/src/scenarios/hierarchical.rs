//! `HierarchicalMembership` with `HierarchicalChange`: enough voters in enough groups, at every
//! level.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use openraft::membership::QuorumSet;
use openraft::membership::RaftMembership;
use openraft_membership::HierarchicalChange;
use openraft_membership::HierarchicalMembership;
use openraft_membership::quorum::Group;

use crate::Policy;
use crate::cluster::Cluster;
use crate::cluster::LEADER;

openraft::declare_raft_types!(
    /// Selects the hierarchical membership and its edit type.
    pub TypeConfig:
        D = types_kv::Request,
        R = types_kv::Response,
        Node = (),
        Membership = HierarchicalMembership<u64, ()>,
        MembershipEdit = HierarchicalChange<u64, ()>,
);

/// Returns three zones of three voters, each with write size 2, under a root group with
/// `root_write_size`.
fn zones(root_write_size: u64) -> Group<u64> {
    Group::of_groups(root_write_size, [
        ("a", Group::of_voters(2, [1, 2, 3])),
        ("b", Group::of_voters(2, [4, 5, 6])),
        ("c", Group::of_voters(2, [7, 8, 9])),
    ])
}

/// Lowers the root write size of three zones from 3 to 2, then isolates zone c. Zones a and b
/// still hold 2 voters each, which is both a read quorum and a write quorum.
pub async fn run() -> anyhow::Result<()> {
    tracing::info!(
        "--- hierarchical: HierarchicalMembership<u64, ()> with HierarchicalChange<u64, ()>. Zones a \
         {{1,2,3}}, b {{4,5,6}} and c {{7,8,9}}, each with write size 2, under a root with write size 3. \
         A write quorum, which commits an entry, reaches every group with its write size: 2 voters in \
         each of the 3 zones. A read quorum, which elects a leader, reaches every group with its read \
         size, member_count - write_size + 1: 2 voters in 1 zone"
    );

    let voters = [1, 2, 3, 4, 5, 6, 7, 8, 9];
    let nodes: BTreeMap<u64, ()> = voters.iter().map(|id| (*id, ())).collect();
    let cluster = Cluster::<TypeConfig>::new(Policy::Hierarchical, voters).await?;
    let leader = cluster.raft(LEADER);

    tracing::info!("--- initialize node 1 with the three zones, under a root with write size 3");
    let mut log_index = {
        let membership = HierarchicalMembership::new(zones(3), nodes.clone())?;

        cluster.call("initialize", leader.initialize(membership)).await?;
        cluster.wait_initialized(LEADER).await?.index()
    };

    tracing::info!(log_index, "--- write k1, and check that every voter holds it");
    let k1 = {
        log_index += 1;
        cluster.write("k1", "v1", log_index, &voters).await?
    };

    tracing::info!(
        log_index,
        "--- lower the root write size to 2 with HierarchicalChange::groups(): a read and a write each \
         need 2 voters in each of 2 zones. The old read quorum {{1,2}} misses the new write quorum \
         {{4,5,7,8}}, so OpenRaft appends a joint config first"
    );
    {
        let edit = HierarchicalChange::groups(zones(2));
        let resp = cluster.call("change_membership", leader.change_membership(edit, false)).await?;

        // The joint config, then the uniform config.
        log_index += 2;
        let membership = HierarchicalMembership::new(zones(2), nodes.clone())?;
        cluster.check_membership(resp, log_index, &membership, &voters).await?;
    }

    tracing::info!(
        log_index,
        "--- write k2, and check that every voter holds it before the isolation"
    );
    let k2 = {
        log_index += 1;
        cluster.write("k2", "v2", log_index, &voters).await?
    };

    tracing::info!(
        log_index,
        "--- isolate zone c: zones a and b hold 2 voters each, a read quorum and a write quorum"
    );
    let k3 = {
        let zone_c = [7, 8, 9];
        cluster.isolate(&zone_c);

        let available = BTreeSet::from([1, 2, 3, 4, 5, 6]);
        let membership = cluster.membership(LEADER);
        let elects = membership.read_quorum_set().is_quorum(available.iter());
        let commits = membership.write_quorum_set().is_quorum(available.iter());
        tracing::info!(
            ?available,
            elects,
            commits,
            "the available voters could elect a new leader, which could then commit its blank entry and writes"
        );
        anyhow::ensure!(
            elects && commits,
            "{:?} must elect and commit in {}",
            available,
            membership
        );

        log_index += 1;
        let k3 = cluster.write("k3", "v3", log_index, &[1, 2, 3, 4, 5, 6]).await?;
        cluster.check_missing(&zone_c, &k3).await?;
        k3
    };

    tracing::info!(
        log_index,
        "--- heal zone c, and check that every voter applies all three writes"
    );
    {
        cluster.heal().await?;
        cluster.wait_applied(&voters, &k3.log_id).await?;
        cluster.check_values(&voters, &[&k1, &k2, &k3]).await?;
    }

    cluster.shutdown().await
}
