use std::collections::BTreeSet;
use std::time::Duration;

use anyhow::Result;
use openraft::RaftMetrics;
use openraft::SnapshotPolicy;
use openraft::alias::EntryOf;
use openraft::entry::RaftEntry;
use openraft::entry::RaftPayload;
use openraft::membership::RaftMembership;
use openraft::membership::RaftMembershipEdit;
use openraft::testing::log_id;
use openraft::type_config::TypeConfigExt;

use crate::fixtures::custom_membership::TestCluster;
use crate::fixtures::custom_membership::TestConfig;
use crate::fixtures::custom_membership::cluster_config;

/// Two membership changes that node 1 makes, starting from `old`. Each membership on the way is
/// given by its `Display`.
///
/// `direct` changes `old` into a membership that OpenRaft appends as one entry: every read quorum
/// of each meets every write quorum of the other. `joint` changes that membership into a target
/// where a read quorum of one misses a write quorum of the other, so OpenRaft appends a joint
/// config first. `joint` turns the voter `removed` into a learner. The target, and the last config
/// of the joint config on the way to it, keep this learner only when the change retains it.
pub struct ChangeScenario<C>
where C: TestConfig
{
    pub node_ids: BTreeSet<u64>,
    pub old: C::Membership,
    pub direct: C::MembershipEdit,
    /// The membership that `direct` makes from `old`.
    pub direct_membership: &'static str,
    pub joint: C::MembershipEdit,
    /// The joint config on the way to the target of `joint` with `removed` as a learner.
    pub joint_with_learner: &'static str,
    /// The joint config on the way to the target of `joint` without `removed`.
    pub joint_without_learner: &'static str,
    /// The target of `joint` with `removed` as a learner.
    pub target_with_learner: &'static str,
    /// The target of `joint` without `removed`.
    pub target_without_learner: &'static str,
    pub removed: u64,
}

impl<C> ChangeScenario<C>
where
    C: TestConfig,
    EntryOf<C>: Clone,
{
    /// Makes both changes through `Raft::change_membership()` with `retain`.
    pub async fn run(self, retain: bool) -> Result<()> {
        let old_membership = self.old.to_string();
        let cluster =
            TestCluster::<C>::start(cluster_config(SnapshotPolicy::Never), self.node_ids.clone(), self.old).await?;

        tracing::info!("--- node 1 wins term 1 and commits its blank entry");
        {
            cluster.raft(1).trigger().elect(false).await?;
            cluster.raft(1).wait(timeout()).applied_index(Some(1), "node 1 commits its blank entry").await?;
        }

        tracing::info!(
            retain,
            "--- OpenRaft appends the result of the direct change at index 2"
        );
        {
            let response = cluster.raft(1).change_membership(self.direct, retain).await?;
            assert_eq!(log_id::<C>(1, 1, 2), response.log_id);
        }

        tracing::info!(
            retain,
            "--- OpenRaft appends the joint config at index 3, then the target at index 4"
        );
        {
            let response = cluster.raft(1).change_membership(self.joint, retain).await?;
            assert_eq!(log_id::<C>(1, 1, 4), response.log_id);
        }

        let (joint_membership, target) = if retain {
            (self.joint_with_learner, self.target_with_learner)
        } else {
            (self.joint_without_learner, self.target_without_learner)
        };

        tracing::info!("--- node 1 applies the old membership, the direct result, the joint config and the target");
        {
            let want = vec![
                (0, old_membership),
                (2, self.direct_membership.to_string()),
                (3, joint_membership.to_string()),
                (4, target.to_string()),
            ];
            let applied = applied_memberships(&cluster, 1);
            assert_eq!(want, applied);
        }

        tracing::info!(
            retain,
            removed = self.removed,
            "--- the target keeps the removed voter as a learner only when retained"
        );
        {
            let mut target_ids = self.node_ids.clone();
            if !retain {
                target_ids.remove(&self.removed);
            }

            for node_id in &target_ids {
                let raft = cluster.raft(*node_id);
                let metrics = raft.wait(timeout()).applied_index(Some(4), "apply the target").await?;
                let membership = metrics.membership_config.membership().to_string();
                assert_eq!(target, membership, "node {node_id}");
            }

            let replicates_to_target = |metrics: &RaftMetrics<C>| {
                let Some(replication) = &metrics.replication else {
                    return false;
                };
                let replicated: BTreeSet<u64> = replication.keys().copied().collect();
                replicated == target_ids
            };
            let raft = cluster.raft(1);
            raft.wait(timeout()).metrics(replicates_to_target, "replicate to the nodes of the target").await?;
        }

        Ok(())
    }

    /// Restarts every node while the joint config on the way to the target of `joint` is in
    /// effect, then leaves the joint config for the target with `removed` as a learner.
    pub async fn restart_in_joint(self) -> Result<()> {
        let old_membership = self.old.to_string();
        let mut config = cluster_config(SnapshotPolicy::Never);
        config.enable_leader_restore = Some(false);
        let cluster = TestCluster::<C>::start(config, self.node_ids.clone(), self.old).await?;

        tracing::info!("--- node 1 wins term 1 and commits its blank entry");
        {
            cluster.raft(1).trigger().elect(false).await?;
            cluster.raft(1).wait(timeout()).applied_index(Some(1), "node 1 commits its blank entry").await?;
        }

        tracing::info!("--- OpenRaft appends the result of the direct change at index 2");
        let direct = {
            let response = cluster.raft(1).change_membership(self.direct, true).await?;
            assert_eq!(log_id::<C>(1, 1, 2), response.log_id);
            response.membership.unwrap()
        };

        tracing::info!("--- node 1 appends the joint config at index 3 as one entry, and every node applies it");
        {
            let target = self.joint.apply(&direct, true).unwrap();
            let joint = direct.joint_with(&target);
            let joint_membership = joint.to_string();
            assert_eq!(self.joint_with_learner, joint_membership);

            let payload = C::Payload::blank();
            let response = cluster.raft(1).append_membership(joint, payload, []).await?;
            assert_eq!(log_id::<C>(1, 1, 3), response.log_id);

            for node_id in &self.node_ids {
                let raft = cluster.raft(*node_id);
                raft.wait(timeout()).applied_index(Some(3), "apply the joint config").await?;
            }
        }

        tracing::info!("--- every node restarts and restores the joint config from its log");
        {
            for node_id in &self.node_ids {
                cluster.restart(*node_id).await?;
            }

            for node_id in &self.node_ids {
                let raft = cluster.raft(*node_id);
                let metrics = raft.wait(timeout()).applied_index(Some(3), "restore the joint config").await?;
                let membership = metrics.membership_config.membership().to_string();
                assert_eq!(self.joint_with_learner, membership, "node {node_id}");
            }
        }

        tracing::info!("--- node 1 wins term 2 and commits a recovery blank entry at index 4");
        {
            let raft = cluster.raft(1);
            raft.trigger().elect(false).await?;
            let recovery_log_id = Some(log_id::<C>(2, 1, 4));
            let commits_recovery = |metrics: &RaftMetrics<C>| metrics.cluster_committed == recovery_log_id;
            raft.wait(timeout()).metrics(commits_recovery, "commit the recovery blank entry").await?;

            let acked_after = C::now();
            raft.trigger().heartbeat().await?;
            raft.wait(timeout())
                .leader_with_quorum_acked(Some(acked_after), "renew the lease of node 1")
                .await?;
        }

        tracing::info!("--- node 1 leaves the joint config for its target at index 5");
        {
            let make_uniform = <C::MembershipEdit as RaftMembershipEdit<C::Membership>>::make_uniform();
            let response = cluster.raft(1).change_membership(make_uniform, true).await?;
            assert_eq!(log_id::<C>(2, 1, 5), response.log_id);

            let want = vec![
                (0, old_membership),
                (2, self.direct_membership.to_string()),
                (3, self.joint_with_learner.to_string()),
                (5, self.target_with_learner.to_string()),
            ];
            let applied = applied_memberships(&cluster, 1);
            assert_eq!(want, applied);
        }

        Ok(())
    }
}

/// Returns the index and the `Display` of each membership that node `node_id` applied.
fn applied_memberships<C>(cluster: &TestCluster<C>, node_id: u64) -> Vec<(u64, String)>
where
    C: TestConfig,
    EntryOf<C>: Clone,
{
    let mut memberships = vec![];
    for entry in cluster.applied(node_id) {
        let Some(membership) = entry.get_membership() else {
            continue;
        };
        let index = entry.index();
        let display = membership.to_string();
        memberships.push((index, display));
    }
    memberships
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2_000))
}
