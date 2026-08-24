use openraft_macros::since;

use crate::Membership;
use crate::node::Node;
use crate::node::NodeId;

/// Defines operations on an entry payload.
#[since(
    version = "0.10.0",
    change = "replaced `C: RaftTypeConfig` with `NID: NodeId, N: Node`"
)]
pub trait RaftPayload<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Return `Some(Membership)` if the entry payload contains a membership payload.
    fn get_membership(&self) -> Option<Membership<NID, N>>;
}
