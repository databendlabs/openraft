//! `WeightedMembership` with `WeightedChange`: voters whose weights reach a threshold.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use openraft::membership::QuorumSet;
use openraft::membership::RaftMembership;
use openraft_membership::WeightedChange;
use openraft_membership::WeightedMembership;

use crate::Policy;
use crate::cluster::Cluster;
use crate::cluster::LEADER;

openraft::declare_raft_types!(
    /// Selects the weighted membership and its edit type.
    pub TypeConfig:
        D = types_kv::Request,
        R = types_kv::Response,
        Node = (),
        Membership = WeightedMembership<u64, ()>,
        MembershipEdit = WeightedChange<u64, ()>,
);

/// Moves the extra weight of voter 1 to voter 2, then isolates voters 3 and 4. Voters 1 and 2
/// weigh 3 of the total 5, which reaches both the read weight and the write weight.
pub async fn run() -> anyhow::Result<()> {
    tracing::info!(
        "--- weighted: WeightedMembership<u64, ()> with WeightedChange<u64, ()>. Weights {{1:2, 2:1, 3:1, \
         4:1}} with write weight 3. Voters that weigh write_weight together are a write quorum, which \
         commits an entry, and voters that weigh total_weight - write_weight + 1 = 3 are a read quorum, \
         which elects a leader"
    );

    let voters = [1, 2, 3, 4];
    let nodes: BTreeMap<u64, ()> = voters.iter().map(|id| (*id, ())).collect();
    let cluster = Cluster::<TypeConfig>::new(Policy::Weighted, voters).await?;
    let leader = cluster.raft(LEADER);

    tracing::info!("--- initialize node 1 with weights {{1:2, 2:1, 3:1, 4:1}} and write weight 3");
    let mut log_index = {
        let weights = BTreeMap::from([(1, 2), (2, 1), (3, 1), (4, 1)]);
        let membership = WeightedMembership::new(weights, 3, nodes.clone())?;

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
        "--- move the extra weight to voter 2 with WeightedChange::weights({{1:1, 2:2, 3:1, 4:1}}), \
         keeping write weight 3. The old write quorum {{1,3}} misses the new read quorum {{2,4}}, so \
         OpenRaft appends a joint config first"
    );
    {
        let weights = BTreeMap::from([(1, 1), (2, 2), (3, 1), (4, 1)]);
        let edit = WeightedChange::weights(weights.clone());
        let resp = cluster.call("change_membership", leader.change_membership(edit, false)).await?;

        // The joint config, then the uniform config.
        log_index += 2;
        let membership = WeightedMembership::new(weights, 3, nodes.clone())?;
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
        "--- isolate voters 3 and 4: voters 1 and 2 weigh 1 + 2 = 3, a read quorum and a write quorum"
    );
    let k3 = {
        cluster.isolate(&[3, 4]);

        let available = BTreeSet::from([1, 2]);
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
        let k3 = cluster.write("k3", "v3", log_index, &[1, 2]).await?;
        cluster.check_missing(&[3, 4], &k3).await?;
        k3
    };

    tracing::info!(
        log_index,
        "--- heal voters 3 and 4, and check that every voter applies all three writes"
    );
    {
        cluster.heal().await?;
        cluster.wait_applied(&voters, &k3.log_id).await?;
        cluster.check_values(&voters, &[&k1, &k2, &k3]).await?;
    }

    cluster.shutdown().await
}
