use std::fmt;

use openraft_macros::since;

use crate::Membership;
use crate::display_ext::DisplayOption;
use crate::log_id::LogId;
use crate::node::Node;
use crate::node::NodeId;
use crate::vote::leader_id::raft_committed_leader_id::RaftCommittedLeaderId;

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
    change = "from `StoredMembership<C>` to `StoredMembership<CLID, NID, N>`"
)]
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct StoredMembership<CLID, NID, N>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
    N: Node,
{
    /// The id of the log that stores this membership config
    log_id: Option<LogId<CLID>>,

    /// Membership config
    membership: Membership<NID, N>,
}

impl<CLID, NID, N> Default for StoredMembership<CLID, NID, N>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
    N: Node,
{
    fn default() -> Self {
        Self {
            log_id: None,
            membership: Membership::default(),
        }
    }
}

impl<CLID, NID, N> StoredMembership<CLID, NID, N>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
    N: Node,
{
    /// Create a new StoredMembership with the given log ID and membership configuration.
    pub fn new(log_id: Option<LogId<CLID>>, membership: Membership<NID, N>) -> Self {
        Self { log_id, membership }
    }

    /// Get the log ID at which this membership was stored.
    pub fn log_id(&self) -> &Option<LogId<CLID>> {
        &self.log_id
    }

    /// Get the membership configuration.
    pub fn membership(&self) -> &Membership<NID, N> {
        &self.membership
    }

    /// Get an iterator over the voter node IDs.
    pub fn voter_ids(&self) -> impl Iterator<Item = NID> {
        self.membership.voter_ids()
    }

    /// Get an iterator over all nodes (ID and node information).
    pub fn nodes(&self) -> impl Iterator<Item = (&NID, &N)> {
        self.membership.nodes()
    }
}

impl<CLID, NID, N> fmt::Display for StoredMembership<CLID, NID, N>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
    N: Node,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{log_id:{}, {}}}", DisplayOption(&self.log_id), self.membership)
    }
}
