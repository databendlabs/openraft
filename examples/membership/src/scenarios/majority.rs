//! The default majority membership: `Membership` with `ChangeMembers`, both from `openraft::impls`.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use openraft::impls::ChangeMembers;
use openraft::impls::Membership;
use openraft::membership::QuorumSet;
use openraft::membership::RaftMembership;

use crate::Policy;
use crate::cluster::Cluster;
use crate::cluster::LEADER;

openraft::declare_raft_types!(
    /// Names the default majority membership and its edit type, which the macro would also pick.
    pub TypeConfig:
        D = types_kv::Request,
        R = types_kv::Response,
        Node = (),
        Membership = Membership<u64, ()>,
        MembershipEdit = ChangeMembers<u64, ()>,
);

/// Adds node 4 to voters `{1,2,3}`, then isolates it. The other 3 of the 4 voters are a majority,
/// so they keep committing writes.
pub async fn run() -> anyhow::Result<()> {
    tracing::info!(
        "--- majority: Membership<u64, ()> with ChangeMembers<u64, ()>. Voters {{1,2,3}}; node 4 joins \
         later. A read quorum, which elects a leader, and a write quorum, which commits an entry, are \
         each a majority of the voters"
    );

    let cluster = Cluster::<TypeConfig>::new(Policy::Majority, [1, 2, 3, 4]).await?;
    let leader = cluster.raft(LEADER);

    tracing::info!("--- initialize node 1 with voters {{1,2,3}}; node 4 starts outside the membership");
    let mut log_index = {
        let voters = BTreeSet::from([1, 2, 3]);
        let nodes = BTreeMap::from([(1, ()), (2, ()), (3, ())]);
        let membership = Membership::new(vec![voters], nodes)?;

        cluster.call("initialize", leader.initialize(membership)).await?;
        cluster.wait_initialized(LEADER).await?.index()
    };

    tracing::info!(log_index, "--- write k1, and check that voters 1, 2 and 3 hold it");
    let k1 = {
        log_index += 1;
        cluster.write("k1", "v1", log_index, &[1, 2, 3]).await?
    };

    tracing::info!(
        log_index,
        "--- add node 4 as a learner, which returns once node 4 has caught up"
    );
    {
        let resp = cluster.call("add_learner", leader.add_learner(4, (), true)).await?;

        log_index += 1;
        let voters = BTreeSet::from([1, 2, 3]);
        let nodes = BTreeMap::from([(1, ()), (2, ()), (3, ()), (4, ())]);
        let membership = Membership::new(vec![voters], nodes)?;
        cluster.check_membership(resp, log_index, &membership, &[1, 2, 3, 4]).await?;
        cluster.check_values(&[4], &[&k1]).await?;
    }

    tracing::info!(
        log_index,
        "--- promote learner 4 with ChangeMembers::AddVoterIds({{4}}). Every majority of {{1,2,3}} \
         meets every majority of {{1,2,3,4}}, so OpenRaft appends the new config without a joint config"
    );
    {
        let edit = ChangeMembers::AddVoterIds(BTreeSet::from([4]));
        let resp = cluster.call("change_membership", leader.change_membership(edit, false)).await?;

        log_index += 1;
        let voters = BTreeSet::from([1, 2, 3, 4]);
        let nodes = BTreeMap::from([(1, ()), (2, ()), (3, ()), (4, ())]);
        let membership = Membership::new(vec![voters], nodes)?;
        cluster.check_membership(resp, log_index, &membership, &[1, 2, 3, 4]).await?;
    }

    tracing::info!(
        log_index,
        "--- write k2, and check that all 4 voters hold it before the isolation"
    );
    let k2 = {
        log_index += 1;
        cluster.write("k2", "v2", log_index, &[1, 2, 3, 4]).await?
    };

    tracing::info!(
        log_index,
        "--- isolate voter 4: voters 1, 2 and 3 are a majority of the 4 voters"
    );
    let k3 = {
        cluster.isolate(&[4]);

        let available = BTreeSet::from([1, 2, 3]);
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
        let k3 = cluster.write("k3", "v3", log_index, &[1, 2, 3]).await?;
        cluster.check_missing(&[4], &k3).await?;
        k3
    };

    tracing::info!(
        log_index,
        "--- heal voter 4, and check that every voter applies all three writes"
    );
    {
        cluster.heal().await?;
        cluster.wait_applied(&[1, 2, 3, 4], &k3.log_id).await?;
        cluster.check_values(&[1, 2, 3, 4], &[&k1, &k2, &k3]).await?;
    }

    cluster.shutdown().await
}
