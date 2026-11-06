//! `ExplicitMembership` with `ExplicitChange`: listed read quorums and listed write quorums.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use openraft::membership::QuorumSet;
use openraft::membership::RaftMembership;
use openraft_membership::ExplicitChange;
use openraft_membership::ExplicitMembership;

use crate::Policy;
use crate::cluster::BLOCKED_WRITE_INTERVAL;
use crate::cluster::Cluster;
use crate::cluster::LEADER;

openraft::declare_raft_types!(
    /// Selects the explicit membership and its edit type.
    pub TypeConfig:
        D = types_kv::Request,
        R = types_kv::Response,
        Node = (),
        Membership = ExplicitMembership<u64, ()>,
        MembershipEdit = ExplicitChange<u64, ()>,
);

/// Restricts the read quorums to `{1,2}`, then isolates backup voter 9. A read quorum is still
/// available, but every write quorum holds voter 9, so a write waits until voter 9 is healed.
pub async fn run() -> anyhow::Result<()> {
    tracing::info!(
        "--- explicit: ExplicitMembership<u64, ()> with ExplicitChange<u64, ()>. The read quorums, which \
         elect a leader, are any 2 of the primary voters {{1,2,3}}. The write quorums, which commit an \
         entry, are each of these pairs plus the backup voter 9"
    );

    let voters = [1, 2, 3, 9];
    let nodes: BTreeMap<u64, ()> = voters.iter().map(|id| (*id, ())).collect();
    let with_backup = vec![
        BTreeSet::from([1, 2, 9]),
        BTreeSet::from([1, 3, 9]),
        BTreeSet::from([2, 3, 9]),
    ];
    let cluster = Cluster::<TypeConfig>::new(Policy::Explicit, voters).await?;
    let leader = cluster.raft(LEADER);

    tracing::info!("--- initialize node 1 with these read and write quorums");
    let mut log_index = {
        let pairs = vec![BTreeSet::from([1, 2]), BTreeSet::from([1, 3]), BTreeSet::from([2, 3])];
        let membership = ExplicitMembership::new(pairs, with_backup.clone(), nodes.clone())?;

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
        "--- restrict the read quorums to {{1,2}} with ExplicitChange::read_quorums(), keeping the write \
         quorums. Every old read quorum meets every write quorum, and so does {{1,2}}, so OpenRaft \
         appends the new config without a joint config"
    );
    {
        let read = vec![BTreeSet::from([1, 2])];
        let edit = ExplicitChange::read_quorums(read.clone());
        let resp = cluster.call("change_membership", leader.change_membership(edit, false)).await?;

        log_index += 1;
        let membership = ExplicitMembership::new(read, with_backup, nodes.clone())?;
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
        "--- isolate backup voter 9: voters 1, 2 and 3 hold the read quorum {{1,2}}, but every write \
         quorum holds voter 9"
    );
    {
        cluster.isolate(&[9]);

        let available = BTreeSet::from([1, 2, 3]);
        let membership = cluster.membership(LEADER);
        let elects = membership.read_quorum_set().is_quorum(available.iter());
        let commits = membership.write_quorum_set().is_quorum(available.iter());
        tracing::info!(
            ?available,
            elects,
            commits,
            "the available voters could elect a new leader, but it could not commit its blank entry, so it \
             would complete no write and no linearizable read"
        );
        anyhow::ensure!(
            elects && !commits,
            "{:?} must elect but not commit in {}",
            available,
            membership
        );
    }

    tracing::info!(
        log_index,
        "--- write k3 while voter 9 is isolated: voters 1, 2 and 3 store it, but it stays pending"
    );
    let k3 = {
        log_index += 1;
        let write = leader.client_write(types_kv::Request::set("k3", "v3"));
        let mut write = std::pin::pin!(write);

        // Poll the write until voters 1, 2 and 3 store it, so that the observation below starts
        // after the leader has proposed it.
        tokio::select! {
            _res = &mut write => anyhow::bail!("write k3 completed while voter 9 was isolated\n{}", cluster.report()),
            res = cluster.wait_log(&[1, 2, 3], log_index) => res?,
        }

        // Keep the write across the timeout: it is pending, not failed.
        let res = tokio::time::timeout(BLOCKED_WRITE_INTERVAL, &mut write).await;
        anyhow::ensure!(
            res.is_err(),
            "write k3 completed while voter 9 was isolated\n{}",
            cluster.report()
        );
        cluster.check_uncommitted(&[1, 2, 3], log_index)?;
        tracing::info!(
            "no response after {:?}, and no voter has committed log index {}",
            BLOCKED_WRITE_INTERVAL,
            log_index
        );

        tracing::info!(
            log_index,
            "--- heal voter 9: it stores k3, which commits and completes the pending write"
        );
        cluster.heal().await?;
        let resp = cluster.call("write k3", write).await?;
        cluster.check_write(resp, "k3", "v3", log_index)?
    };

    tracing::info!(log_index, "--- check that every voter applies all three writes");
    {
        cluster.wait_applied(&voters, &k3.log_id).await?;
        cluster.check_values(&voters, &[&k1, &k2, &k3]).await?;
    }

    cluster.shutdown().await
}
