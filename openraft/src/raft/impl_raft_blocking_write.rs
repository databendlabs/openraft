//! Implement blocking-mode write operations for Raft.
//! Blocking-mode write API blocks until the write operation is completed,
//! where [`RaftTypeConfig::Responder`] is a [`OneshotResponder`].

use crate::error::into_raft_result::IntoRaftResult;
use crate::error::ClientWriteError;
use crate::error::RaftError;
#[cfg(doc)]
use crate::impls::OneshotResponder;
use crate::raft::ClientWriteResponse;
#[cfg(doc)]
use crate::raft::ManagementApi;
use crate::ChangeMembers;
use crate::Raft;
use crate::RaftTypeConfig;

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
