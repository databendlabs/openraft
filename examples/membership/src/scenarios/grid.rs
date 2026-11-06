//! `GridMembership` with `GridChange`: a complete row elects a leader, and a voter of each row
//! commits an entry.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use openraft::membership::QuorumSet;
use openraft::membership::RaftMembership;
use openraft_membership::GridChange;
use openraft_membership::GridMembership;

use crate::Policy;
use crate::cluster::Cluster;
use crate::cluster::LEADER;

openraft::declare_raft_types!(
    /// Selects the grid membership and its edit type.
    pub TypeConfig:
        D = types_kv::Request,
        R = types_kv::Response,
        Node = (),
        Membership = GridMembership<u64, ()>,
        MembershipEdit = GridChange<u64, ()>,
);

/// Transposes a 3 x 3 grid, then isolates every voter but 1, 5 and 9. Each new row holds one of
/// them, so they commit, but no new row is complete, so they could not elect a leader.
pub async fn run() -> anyhow::Result<()> {
    tracing::info!(
        "--- grid: GridMembership<u64, ()> with GridChange<u64, ()>. Rows [[1,2,3], [4,5,6], [7,8,9]]. \
         A voter of every row is a write quorum, which commits an entry, and a complete row is a read \
         quorum, which elects a leader"
    );

    let voters = [1, 2, 3, 4, 5, 6, 7, 8, 9];
    let nodes: BTreeMap<u64, ()> = voters.iter().map(|id| (*id, ())).collect();
    let cluster = Cluster::<TypeConfig>::new(Policy::Grid, voters).await?;
    let leader = cluster.raft(LEADER);

    tracing::info!("--- initialize node 1 with rows [[1,2,3], [4,5,6], [7,8,9]]");
    let mut log_index = {
        let grid = vec![vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 9]];
        let membership = GridMembership::new(grid, nodes.clone())?;

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
        "--- transpose the grid to rows [[1,4,7], [2,5,8], [3,6,9]] with GridChange::grid(). The old row \
         {{1,2,3}} misses the new write quorum {{4,5,6}}, so OpenRaft appends a joint config first"
    );
    {
        let grid = vec![vec![1, 4, 7], vec![2, 5, 8], vec![3, 6, 9]];
        let edit = GridChange::grid(grid.clone());
        let resp = cluster.call("change_membership", leader.change_membership(edit, false)).await?;

        // The joint config, then the uniform config.
        log_index += 2;
        let membership = GridMembership::new(grid, nodes.clone())?;
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
        "--- isolate every voter but 1, 5 and 9: they hold a voter of each new row, but no complete row"
    );
    let k3 = {
        let isolated = [2, 3, 4, 6, 7, 8];
        cluster.isolate(&isolated);

        let available = BTreeSet::from([1, 5, 9]);
        let membership = cluster.membership(LEADER);
        let elects = membership.read_quorum_set().is_quorum(available.iter());
        let commits = membership.write_quorum_set().is_quorum(available.iter());
        tracing::info!(
            ?available,
            elects,
            commits,
            "leader 1 keeps committing with one copy per row, but an election needs a complete row"
        );
        anyhow::ensure!(
            !elects && commits,
            "{:?} must commit but not elect in {}",
            available,
            membership
        );

        log_index += 1;
        let k3 = cluster.write("k3", "v3", log_index, &[1, 5, 9]).await?;
        cluster.check_missing(&isolated, &k3).await?;
        k3
    };

    tracing::info!(
        log_index,
        "--- heal the isolated voters, and check that every voter applies all three writes"
    );
    {
        cluster.heal().await?;
        cluster.wait_applied(&voters, &k3.log_id).await?;
        cluster.check_values(&voters, &[&k1, &k2, &k3]).await?;
    }

    cluster.shutdown().await
}
