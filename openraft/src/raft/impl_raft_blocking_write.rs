//! Implement blocking-mode write operations for Raft.
//! Blocking-mode write API blocks until the write operation is completed,
//! where [`RaftTypeConfig::Responder`] is a [`OneshotResponder`].

use openraft_macros::since;

use crate::ChangeMembers;
use crate::Membership;
use crate::Raft;
use crate::RaftTypeConfig;
use crate::batch::Batch;
use crate::errors::ClientWriteError;
use crate::errors::RaftError;
use crate::errors::into_raft_result::IntoRaftResult;
#[cfg(doc)]
use crate::impls::OneshotResponder;
use crate::raft::ChangeMembershipOutcome;
use crate::raft::ChangeMembershipRequest;
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

    /// Propose a cluster configuration change from a [`ChangeMembershipRequest`].
    ///
    /// A request without an application-defined payload starts each physical log entry from a
    /// separate `C::Payload::blank()`. [`ChangeMembershipRequest::with_payload()`] supplies
    /// separate application-defined bases instead. OpenRaft calls `with_membership()` on each
    /// base payload with the membership computed for that physical log entry.
    ///
    /// Request preconditions follow the same two-step rules as [`Self::change_membership_if()`].
    /// They guard the first proposal. OpenRaft updates the membership-log precondition for a
    /// possible uniform proposal and carries over only the committed-leader precondition.
    ///
    /// A voter change may append a joint entry followed by a uniform entry. `with_payload()`
    /// accepts one payload for the requested change and another for the possible uniform entry.
    /// If `with_membership()` preserves application data, the state machine applies each
    /// payload at its corresponding log ID.
    ///
    /// Both supplied payloads are serialized, stored, and replicated when the change requires two
    /// physical entries.
    ///
    /// If the uniform proposal fails, the joint entry is already committed. A retry must build a
    /// new request with the original payloads and preconditions based on the current joint state.
    ///
    /// The returned [`ChangeMembershipOutcome`] contains the first response and the uniform
    /// response when the change entered joint consensus.
    #[since(
        version = "0.10.0",
        change = "accept a request with an optional payload and preconditions"
    )]
    #[since(version = "0.10.0", change = "added payload-aware membership change API")]
    #[tracing::instrument(level = "info", skip_all)]
    pub async fn change_membership_with_payload(
        &self,
        request: ChangeMembershipRequest<C>,
    ) -> Result<ChangeMembershipOutcome<C>, RaftError<C, ClientWriteError<C>>> {
        self.management_api().change_membership_with_payload(request).await.into_raft_result()
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

    /// Append a caller-built membership as one log entry, with no intermediate joint membership.
    ///
    /// This is Raft's single-step (single-server) membership change, generalized: besides the
    /// one-voter step it also accepts a caller-built joint membership. It is not the default way
    /// to change membership. The caller supplies the whole stored membership here — every voter
    /// set, every learner and every node's metadata — and openraft checks only the transition
    /// rule below. Prefer [`Self::change_membership()`], which derives all of that from the
    /// current membership, unless you know exactly what this method will write.
    ///
    /// Unlike [`Self::change_membership()`], which computes a joint config and then flattens it in
    /// a second entry, this method writes exactly the `membership` given, in one physical log
    /// entry. It never adds a joint membership and never flattens one, so a caller-built joint
    /// membership of any number of voter sets is stored as it is. It returns after that entry is
    /// committed and applied.
    ///
    /// `payload` is the base entry payload. This method calls
    /// [`RaftPayload::with_membership()`] on it, and that implementation decides how much of the
    /// base survives: if it preserves application data, the data and the membership are stored
    /// and applied at one log id. The default `EntryPayload::with_membership()` returns
    /// `EntryPayload::Membership`, so it replaces an `EntryPayload::Normal(data)` base and the
    /// state machine never receives `data`. A caller with no application data passes
    /// `C::Payload::blank()`.
    ///
    /// # Accepted transitions
    ///
    /// The proposed membership must be reachable from the last committed one by a transition whose
    /// quorum intersection openraft can prove cheaply:
    ///
    /// - Both memberships are uniform, and their voter sets differ by at most one node id.
    /// - Otherwise, the two memberships share an exactly equal voter set.
    ///
    /// Anything else fails with [`UnsupportedMembershipTransition`], which is a limit of this
    /// rule, not proof that the transition is unsafe.
    ///
    /// # Rejections that write no log
    ///
    /// - [`InProgress`], while the preceding membership is not committed yet.
    /// - [`UncommittedLeaderLog`], until this leader commits a log entry of its own term. This
    ///   holds even after the preceding membership commits and the leader holds a valid lease.
    /// - [`NodeMetadataChanged`], when the proposed membership gives a different `Node` to a node
    ///   id the cluster already knows. Adding and removing a node stay allowed; an intentional
    ///   metadata update is still [`ChangeMembers::SetNodes`].
    /// - [`ClientWriteError::PreconditionFailed`], when any supplied [`Precondition`] does not
    ///   hold. All preconditions are checked before anything is validated or written.
    ///
    /// A caller that derives the proposed membership from an observed one must pass
    /// [`Precondition::LastMembershipLogId`] with that membership's log id. Without it, a
    /// membership change proposed in between is silently overwritten.
    ///
    /// # Examples
    ///
    /// Promote learner `4` to a voter of `{1,2,3}`. Node `4` must already be a learner, so
    /// that `observed` carries its `Node`: `Membership::new()` returns `NodeNotFound` for a
    /// voter whose metadata is absent.
    ///
    /// ```ignore
    /// let observed = raft.metrics().borrow().membership_config.clone();
    ///
    /// let voters = BTreeSet::from([1, 2, 3, 4]);
    /// let nodes = observed.membership().nodes().map(|(id, n)| (id.clone(), n.clone()));
    /// let proposed = Membership::new(vec![voters], nodes.collect::<BTreeMap<_, _>>())?;
    ///
    /// raft.append_membership(proposed, MyPayload::blank(), [Precondition::LastMembershipLogId {
    ///     last_membership_log_id: observed.log_id().clone(),
    /// }])
    /// .await?;
    /// ```
    ///
    /// Move from the uniform `{1,2,3}` to a joint membership of three voter sets, where nodes `4`
    /// through `7` are already learners. It is accepted because `{1,2,3}` appears in both, exactly
    /// equal:
    ///
    /// ```ignore
    /// let observed = raft.metrics().borrow().membership_config.clone();
    ///
    /// let configs = vec![
    ///     BTreeSet::from([1, 2, 3]),
    ///     BTreeSet::from([3, 4, 5]),
    ///     BTreeSet::from([5, 6, 7]),
    /// ];
    /// let nodes = observed.membership().nodes().map(|(id, n)| (id.clone(), n.clone()));
    /// let proposed = Membership::new(configs, nodes.collect::<BTreeMap<_, _>>())?;
    ///
    /// raft.append_membership(proposed, MyPayload::blank(), [Precondition::LastMembershipLogId {
    ///     last_membership_log_id: observed.log_id().clone(),
    /// }])
    /// .await?;
    /// ```
    ///
    /// [`RaftPayload::with_membership()`]: crate::entry::RaftPayload::with_membership
    /// [`UnsupportedMembershipTransition`]: crate::errors::UnsupportedMembershipTransition
    /// [`InProgress`]: crate::errors::InProgress
    /// [`UncommittedLeaderLog`]: crate::errors::UncommittedLeaderLog
    /// [`NodeMetadataChanged`]: crate::errors::NodeMetadataChanged
    #[since(version = "0.10.0")]
    #[tracing::instrument(level = "info", skip_all)]
    pub async fn append_membership(
        &self,
        membership: Membership<C::NodeId, C::Node>,
        payload: C::Payload,
        preconditions: impl IntoIterator<Item = Precondition<C>>,
    ) -> Result<ClientWriteResponse<C>, RaftError<C, ClientWriteError<C>>> {
        self.management_api()
            .append_membership(membership, payload, BatchOf::<C, _>::of(preconditions))
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
