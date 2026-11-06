//! `ThresholdMembership` with `ThresholdChange`: any `write_size` voters commit an entry.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use openraft::membership::QuorumSet;
use openraft::membership::RaftMembership;
use openraft_membership::ThresholdChange;
use openraft_membership::ThresholdMembership;

use crate::Policy;
use crate::cluster::Cluster;
use crate::cluster::LEADER;

openraft::declare_raft_types!(
    /// Selects the threshold membership and its edit type.
    pub TypeConfig:
        D = types_kv::Request,
        R = types_kv::Response,
        Node = (),
        Membership = ThresholdMembership<u64, ()>,
        MembershipEdit = ThresholdChange<u64, ()>,
);

/// Lowers the write size of voters `{1,2,3,4,5}` from 3 to 2, then isolates voters 3, 4 and 5.
/// Voters 1 and 2 still commit, but they hold no read quorum, so they could not elect a leader.
pub async fn run() -> anyhow::Result<()> {
    tracing::info!(
        "--- threshold: ThresholdMembership<u64, ()> with ThresholdChange<u64, ()>. Voters {{1,2,3,4,5}} \
         with write size 3. Any write_size voters are a write quorum, which commits an entry, and any \
         voter_count + 1 - write_size = 3 voters are a read quorum, which elects a leader"
    );

    let voters = [1, 2, 3, 4, 5];
    let nodes: BTreeMap<u64, ()> = voters.iter().map(|id| (*id, ())).collect();
    let cluster = Cluster::<TypeConfig>::new(Policy::Threshold, voters).await?;
    let leader = cluster.raft(LEADER);

    tracing::info!("--- initialize node 1 with voters {{1,2,3,4,5}} and write size 3");
    let mut log_index = {
        let membership = ThresholdMembership::new(BTreeSet::from(voters), 3, nodes.clone())?;

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
        "--- lower the write size to 2 with ThresholdChange::write_size(2), which raises the read size to \
         4. The old read quorum {{1,2,3}} misses the new write quorum {{4,5}}, so OpenRaft appends a joint \
         config first"
    );
    {
        let edit = ThresholdChange::write_size(2);
        let resp = cluster.call("change_membership", leader.change_membership(edit, false)).await?;

        // The joint config, then the uniform config.
        log_index += 2;
        let membership = ThresholdMembership::new(BTreeSet::from(voters), 2, nodes.clone())?;
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
        "--- isolate voters 3, 4 and 5: voters 1 and 2 are a write quorum, but no read quorum"
    );
    let k3 = {
        cluster.isolate(&[3, 4, 5]);

        let available = BTreeSet::from([1, 2]);
        let membership = cluster.membership(LEADER);
        let elects = membership.read_quorum_set().is_quorum(available.iter());
        let commits = membership.write_quorum_set().is_quorum(available.iter());
        tracing::info!(
            ?available,
            elects,
            commits,
            "leader 1 keeps committing with 2 copies, but an election needs 4 votes"
        );
        anyhow::ensure!(
            !elects && commits,
            "{:?} must commit but not elect in {}",
            available,
            membership
        );

        log_index += 1;
        let k3 = cluster.write("k3", "v3", log_index, &[1, 2]).await?;
        cluster.check_missing(&[3, 4, 5], &k3).await?;
        k3
    };

    tracing::info!(
        log_index,
        "--- heal voters 3, 4 and 5, and check that every voter applies all three writes"
    );
    {
        cluster.heal().await?;
        cluster.wait_applied(&voters, &k3.log_id).await?;
        cluster.check_values(&voters, &[&k1, &k2, &k3]).await?;
    }

    cluster.shutdown().await
}
