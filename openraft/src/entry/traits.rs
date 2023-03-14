use crate::log_id::RaftLogId;
use crate::Membership;
use crate::Node;
use crate::NodeId;

/// Defines operations on an entry payload.
pub trait RaftPayload<NID, N>
where
    N: Node,
    NID: NodeId,
{
    /// Return `true` if the entry payload is blank.
    fn is_blank(&self) -> bool;

    /// Return `Some(&Membership)` if the entry payload is a membership payload.
    fn get_membership(&self) -> Option<&Membership<NID, N>>;
}

/// Defines operations on an entry.
pub trait RaftEntry<NID, N>: RaftPayload<NID, N> + RaftLogId<NID>
where
    N: Node,
    NID: NodeId,
{
}
