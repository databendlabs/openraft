use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

use display_more::DisplayOptionExt;
use openraft_macros::since;

use crate::Membership;
use crate::errors::ErrorSource;
use crate::log_id::LogId;
use crate::membership::RaftMembership;
use crate::membership::RaftMembershipExt;
use crate::node::Node;
use crate::node::NodeId;
use crate::quorum::QuorumSet;
use crate::vote::RaftCommittedLeaderId;

/// This struct represents information about a membership config that has already been stored in the
/// raft logs.
///
/// It includes log id and a membership config. Such a record is used in the state machine or
/// snapshot to track the last membership and its log id. And it is also used as a return value for
/// functions that return membership and its log position.
///
/// It derives `Default` for building an uninitialized membership state, e.g., when a raft-node is
/// just created.
#[since(
    version = "0.10.0",
    change = "membership helpers and QuorumSet support custom error sources"
)]
#[since(
    version = "0.10.0",
    change = "from `StoredMembership<C>` to `StoredMembership<CLID, Mem>`"
)]
#[since(version = "0.8.0")]
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct StoredMembership<CLID, Mem>
where
    CLID: RaftCommittedLeaderId,
    Mem: RaftMembership,
{
    /// The id of the log that stores this membership config
    log_id: Option<LogId<CLID>>,

    /// Membership config
    membership: Mem,
}

impl<CLID, Mem> Default for StoredMembership<CLID, Mem>
where
    CLID: RaftCommittedLeaderId,
    Mem: RaftMembership,
{
    fn default() -> Self {
        Self {
            log_id: None,
            membership: Mem::default(),
        }
    }
}

impl<CLID, Mem> StoredMembership<CLID, Mem>
where
    CLID: RaftCommittedLeaderId,
    Mem: RaftMembership,
{
    /// Create a new StoredMembership with the given log ID and membership configuration.
    pub fn new(log_id: Option<LogId<CLID>>, membership: Mem) -> Self {
        Self { log_id, membership }
    }

    pub(crate) fn new_arc(log_id: Option<LogId<CLID>>, membership: Mem) -> Arc<Self> {
        Arc::new(Self::new(log_id, membership))
    }

    /// Get the log ID at which this membership was stored.
    pub fn log_id(&self) -> &Option<LogId<CLID>> {
        &self.log_id
    }

    /// Get the membership configuration.
    pub fn membership(&self) -> &Mem {
        &self.membership
    }

    /// Returns whether `nid` may start an election under the stored membership configuration.
    ///
    /// The log ID does not affect campaign eligibility.
    pub(crate) fn is_campaigner(&self, nid: &Mem::NodeId) -> bool {
        self.membership.is_campaigner(nid)
    }

    /// Get an iterator over the voter node IDs.
    pub fn voter_ids(&self) -> impl Iterator<Item = Mem::NodeId> {
        self.membership.voter_ids()
    }

    /// Returns known node IDs outside the write voter configurations.
    pub(crate) fn non_write_voter_ids(&self) -> impl Iterator<Item = Mem::NodeId> + '_ {
        self.membership.non_write_voter_ids()
    }

    /// Get the node (either voter or learner) by node id.
    #[since(version = "0.10.0")]
    pub fn get_node(&self, node_id: &Mem::NodeId) -> Option<&Mem::Node> {
        let node_ref = self.membership.get_node(node_id)?;
        Some(node_ref.node)
    }
}

impl<CLID, NID, N, E> StoredMembership<CLID, Membership<NID, N, E>>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
    N: Node,
    E: ErrorSource,
{
    /// Get an iterator over all nodes (ID and node information).
    pub fn nodes(&self) -> impl Iterator<Item = (&NID, &N)> {
        self.membership.nodes()
    }

    /// Returns reference to the joint config.
    ///
    /// Membership is defined by a joint of multiple configs.
    /// Each config is a set of node-id.
    #[since(version = "0.10.0")]
    pub fn get_joint_config(&self) -> &Vec<BTreeSet<NID>> {
        self.membership.get_joint_config()
    }
}

impl<CLID, Mem> fmt::Display for StoredMembership<CLID, Mem>
where
    CLID: RaftCommittedLeaderId,
    Mem: RaftMembership,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{log_id:{}, {}}}", self.log_id.display(), self.membership)
    }
}

/// Implement node-id joint quorum set.
impl<CLID, NID, N, E> QuorumSet for StoredMembership<CLID, Membership<NID, N, E>>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
    N: Node,
    E: ErrorSource,
{
    type Id = NID;
    type Iter = std::collections::btree_set::IntoIter<NID>;

    fn is_quorum<'a, I: Iterator<Item = &'a NID> + Clone>(&self, ids: I) -> bool {
        self.membership.is_quorum(ids)
    }

    fn ids(&self) -> Self::Iter {
        self.membership.ids()
    }
}
