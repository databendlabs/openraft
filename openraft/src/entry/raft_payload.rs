use openraft_macros::since;

use crate::Membership;
use crate::MembershipMetadata;
use crate::node::Node;
use crate::node::NodeId;

/// Defines operations on an entry payload.
#[since(
    version = "0.10.0",
    change = "replaced `C: RaftTypeConfig` with `NID: NodeId, N: Node, M: MembershipMetadata`"
)]
pub trait RaftPayload<NID, N, M = ()>
where
    NID: NodeId,
    N: Node,
    M: MembershipMetadata,
{
    /// Return `Some(Membership)` if the entry payload contains a membership payload.
    fn get_membership(&self) -> Option<Membership<NID, N, M>>;
}
