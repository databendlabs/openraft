use std::fmt;

use display_more::DisplayOptionExt;
use openraft_macros::since;

use crate::SnapshotId;
use crate::StoredMembership;
use crate::log_id::LogId;
use crate::node::Node;
use crate::node::NodeId;
use crate::storage::SnapshotSignature;
use crate::vote::RaftCommittedLeaderId;

/// The metadata of a snapshot.
///
/// Including the last log id that is included in this snapshot,
/// the last membership included,
/// and a snapshot id.
#[since(
    version = "0.10.0",
    change = "from `SnapshotMeta<C>` to `SnapshotMeta<CLID, NID, N>`"
)]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct SnapshotMeta<CLID, NID, N>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
    N: Node,
{
    /// Log entries up to which this snapshot includes, inclusive.
    pub last_log_id: Option<LogId<CLID>>,

    /// The last applied membership config.
    pub last_membership: StoredMembership<CLID, NID, N>,

    /// To identify a snapshot when transferring.
    /// Caveat: even when two snapshots are built with the same `last_log_id`, they still could be
    /// different in bytes.
    pub snapshot_id: SnapshotId,
}

impl<CLID, NID, N> Default for SnapshotMeta<CLID, NID, N>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
    N: Node,
{
    fn default() -> Self {
        Self {
            last_log_id: None,
            last_membership: StoredMembership::default(),
            snapshot_id: SnapshotId::default(),
        }
    }
}

impl<CLID, NID, N> fmt::Display for SnapshotMeta<CLID, NID, N>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
    N: Node,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{{snapshot_id: {}, last_log:{}, last_membership: {}}}",
            self.snapshot_id,
            self.last_log_id.display(),
            self.last_membership
        )
    }
}

impl<CLID, NID, N> SnapshotMeta<CLID, NID, N>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
    N: Node,
{
    /// Get the signature of this snapshot metadata for comparison and identification.
    pub fn signature(&self) -> SnapshotSignature<CLID> {
        SnapshotSignature {
            last_log_id: self.last_log_id.clone(),
            last_membership_log_id: self.last_membership.log_id().as_ref().map(|x| Box::new(x.clone())),
            snapshot_id: self.snapshot_id.clone(),
        }
    }

    /// Returns a ref to the id of the last log that is included in this snapshot.
    pub fn last_log_id(&self) -> Option<&LogId<CLID>> {
        self.last_log_id.as_ref()
    }
}
