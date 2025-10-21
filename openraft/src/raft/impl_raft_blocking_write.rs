//! Implement blocking-mode write operations for Raft.
//! Blocking-mode write API blocks until the write operation is completed,
//! where [`RaftTypeConfig::Responder`] is a [`OneshotResponder`].

use crate::ChangeMembers;
use crate::Raft;
use crate::RaftTypeConfig;
use crate::error::ClientWriteError;
use crate::error::RaftError;
use crate::error::into_raft_result::IntoRaftResult;
#[cfg(doc)]
use crate::impls::OneshotResponder;
use crate::raft::ClientWriteResponse;
#[cfg(doc)]
use crate::raft::ManagementApi;

/// Implement blocking mode write operations those reply on oneshot channel for communication
/// between Raft core and client.
impl<C> Raft<C>
where C: RaftTypeConfig
{
    /// Propose a cluster configuration change.
    ///
    /// A node in the proposed config has to be a learner, otherwise it fails with LearnerNotFound
    /// error.
    ///
    /// Internally:
    /// - It proposes a **joint** config.
    /// - When the **joint** config is committed, it proposes a uniform config.
    ///
    /// Read more about the behavior of [joint
    /// consensus](crate::docs::cluster_control::joint_consensus).
    ///
    /// If `retain` is `true`, then all the members not existing in the new membership
    /// will be turned into learners, otherwise will be removed.
    /// If `retain` is `false`, the removed voter will be removed from the cluster.
    /// Existing learners will not be affected.
    ///
    /// Example of `retain` usage:
    /// If the original membership is `{"voter":{1,2,3}, "nodes":{1,2,3,4,5}}`, where `nodes`
    /// includes node information of both voters and learners. In this case, `4,5` are learners.
    /// Call `change_membership` with `voters={2,3,4}`, then:
    ///    - If `retain` is `true`, the committed new membership is
    ///     `{"voters":{2,3,4}, "nodes":{1,2,3,4,5}}`, node `1` is turned into a learner.
    ///    - Otherwise if `retain` is `false`, then the new membership is `{"voters":{2,3,4},
    ///      "nodes":{2,3,4,5}}`, in which the removed voters `1` are removed from the cluster. `5`
    ///      is not affected.
    ///
    /// If it loses leadership or crashed before committing the second **uniform** config log, the
    /// cluster is left in the **joint** config.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use std::collections::BTreeSet;
    ///
    /// // Change membership to nodes {2, 3, 4}, keeping removed voters as learners
    /// let new_voters = BTreeSet::from([2, 3, 4]);
    /// raft.change_membership(new_voters, true).await?;
    ///
    /// // Change membership to nodes {3, 4, 5}, removing node 2 from cluster
    /// let new_voters = BTreeSet::from([3, 4, 5]);
    /// raft.change_membership(new_voters, false).await?;
    /// ```
    #[tracing::instrument(level = "info", skip_all)]
    pub async fn change_membership(
        &self,
        members: impl Into<ChangeMembers<C>>,
        retain: bool,
    ) -> Result<ClientWriteResponse<C>, RaftError<C, ClientWriteError<C>>> {
        self.management_api().change_membership(members, retain).await.into_raft_result()
    }

    /// Add a new learner raft node, optionally, blocking until up-to-speed.
    ///
    /// - Add a node as learner into the cluster.
    /// - Setup replication from leader to it.
    ///
    /// If `blocking` is `true`, this function blocks until the leader believes the logs on the new
    /// node are up to date, i.e., ready to join the cluster, as a voter, by calling
    /// `change_membership`.
    ///
    /// If blocking is `false`, this function returns at once as successfully setting up the
    /// replication.
    ///
    /// If the node to add is already a voter or learner, it will still re-add it.
    ///
    /// A `node` is able to store the network address of a node. Thus, an application does not
    /// need another store for mapping node-id to ip-addr when implementing the RaftNetwork.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use openraft::BasicNode;
    ///
    /// // Add node 4 as a learner (non-blocking)
    /// let node = BasicNode { addr: "127.0.0.1:8083".to_string() };
    /// raft.add_learner(4, node, false).await?;
    ///
    /// // Add node 5 as a learner and wait for it to catch up (blocking)
    /// let node = BasicNode { addr: "127.0.0.1:8084".to_string() };
    /// raft.add_learner(5, node, true).await?;
    /// ```
    #[tracing::instrument(level = "debug", skip(self, id), fields(target=display(&id)))]
    pub async fn add_learner(
        &self,
        id: C::NodeId,
        node: C::Node,
        blocking: bool,
    ) -> Result<ClientWriteResponse<C>, RaftError<C, ClientWriteError<C>>> {
        self.management_api().add_learner(id, node, blocking).await.into_raft_result()
    }
}
