//! Implement blocking-mode write operations for Raft.
//! Blocking-mode write API blocks until the write operation is completed,
//! where [`RaftTypeConfig::Responder`] is a [`OneshotResponder`].

use openraft_macros::since;

use crate::ChangeMembers;
use crate::Raft;
use crate::RaftTypeConfig;
use crate::batch::Batch;
use crate::errors::ClientWriteError;
use crate::errors::RaftError;
use crate::errors::into_raft_result::IntoRaftResult;
#[cfg(doc)]
use crate::impls::OneshotResponder;
use crate::raft::ChangeMembershipOutcome;
use crate::raft::ClientWriteResponse;
#[cfg(doc)]
use crate::raft::ManagementApi;
use crate::raft::Precondition;
use crate::storage::RaftStateMachine;
use crate::type_config::alias::BatchOf;

/// Implement blocking mode write operations those reply on oneshot channel for communication
/// between Raft core and client.
impl<C, SM> Raft<C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
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
        members: impl Into<ChangeMembers<C::NodeId, C::Node>>,
        retain: bool,
    ) -> Result<ClientWriteResponse<C>, RaftError<C, ClientWriteError<C>>> {
        self.management_api()
            .change_membership(members, retain, BatchOf::<C, _>::of([]))
            .await
            .into_raft_result()
    }

    /// Propose a cluster configuration change with an application-defined payload.
    ///
    /// OpenRaft replaces any membership already stored in `payload` with the membership computed
    /// for each physical log entry.
    ///
    /// A voter change may append a joint entry followed by a uniform entry. This method clones
    /// `payload` before the first proposal, so both entries start from the same application
    /// payload. If `with_membership()` preserves application data, the state machine applies that
    /// data at two different log IDs. Non-idempotent data therefore needs an application-level
    /// change identifier or deduplication.
    ///
    /// Each physical entry serializes, stores, and replicates its payload. A large payload may use
    /// log and network space twice.
    ///
    /// If the uniform proposal fails, the joint entry is already committed. A retry must call this
    /// method again with the original payload.
    ///
    /// The returned [`ChangeMembershipOutcome`] contains the first response and the uniform
    /// response when the change entered joint consensus.
    #[since(version = "0.10.0", change = "added payload-aware membership change API")]
    #[tracing::instrument(level = "info", skip_all)]
    pub async fn change_membership_with_payload(
        &self,
        members: impl Into<ChangeMembers<C::NodeId, C::Node>>,
        retain: bool,
        payload: C::Payload,
    ) -> Result<ChangeMembershipOutcome<C>, RaftError<C, ClientWriteError<C>>>
    where
        C::Payload: Clone,
    {
        let first_payload = payload.clone();
        let uniform_payload = move || payload;
        let preconditions = BatchOf::<C, _>::of([]);
        let api = self.management_api();
        let result = api
            .change_membership_with_payloads(members, retain, preconditions, first_payload, uniform_payload)
            .await;
        result.into_raft_result()
    }

    /// Propose a cluster configuration change only if every [`Precondition`] is satisfied.
    ///
    /// This is [`Self::change_membership()`] with a compare-and-set guard: the Raft core checks
    /// every precondition against its state before proposing the change, and fails with
    /// [`ClientWriteError::PreconditionFailed`] if any of them does not hold.
    ///
    /// Passing [`Precondition::LastMembershipLogId`] serializes concurrent membership changes:
    /// the change is proposed only while the effective membership is still the one the caller
    /// based its decision on.
    ///
    /// # Preconditions and the two-step change
    ///
    /// A voter change is proposed in two steps, a **joint** config and then a **uniform** config
    /// that flattens it, and the preconditions given here guard only the first. Appending the
    /// joint entry moves the last log id and the effective membership log id past what the caller
    /// observed, so only [`Precondition::CommittedLeaderId`] carries over; openraft re-guards the
    /// flattening step itself with the joint entry's log id.
    ///
    /// A [`ClientWriteError::PreconditionFailed`] from the flattening step therefore does not mean
    /// nothing was written: the joint config is already committed and the cluster is left in joint
    /// consensus, as it is when leadership is lost mid-change. Its `expected` field then holds the
    /// joint entry's log id, a value the caller never passed.
    #[since(version = "0.10.0")]
    #[tracing::instrument(level = "info", skip_all)]
    pub async fn change_membership_if(
        &self,
        members: impl Into<ChangeMembers<C::NodeId, C::Node>>,
        retain: bool,
        preconditions: impl IntoIterator<Item = Precondition<C>>,
    ) -> Result<ClientWriteResponse<C>, RaftError<C, ClientWriteError<C>>> {
        self.management_api()
            .change_membership(members, retain, BatchOf::<C, _>::of(preconditions))
            .await
            .into_raft_result()
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
