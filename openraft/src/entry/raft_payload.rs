use std::fmt::Debug;
use std::fmt::Display;

use openraft_macros::since;

use crate::AppData;
use crate::Membership;
use crate::base::OptionalFeatures;
use crate::node::Node;
use crate::node::NodeId;

/// Defines operations for constructing and inspecting an entry payload.
#[since(
    version = "0.10.0",
    change = "replaced generic parameters with associated types and added construction methods"
)]
#[since(
    version = "0.10.0",
    change = "replaced `C: RaftTypeConfig` with `NID: NodeId, N: Node`"
)]
pub trait RaftPayload
where Self: OptionalFeatures + Debug + Display + Sized + 'static
{
    /// Application-specific data stored in the payload.
    #[since(version = "0.10.0")]
    type D: AppData;

    /// The node ID type used in memberships.
    #[since(version = "0.10.0")]
    type NodeId: NodeId;

    /// The node type used in memberships.
    #[since(version = "0.10.0")]
    type Node: Node;

    /// Create a blank payload.
    #[since(version = "0.10.0")]
    fn blank() -> Self;

    /// Replace the normal application data in this payload.
    #[since(version = "0.10.0")]
    fn with_normal(self, data: Self::D) -> Self;

    /// Replace the membership in this payload.
    #[since(version = "0.10.0")]
    fn with_membership(self, membership: Membership<Self::NodeId, Self::Node>) -> Self;

    /// Return `Some(Membership)` if the entry payload contains a membership payload.
    #[since(version = "0.10.0", change = "use associated node types")]
    fn get_membership(&self) -> Option<Membership<Self::NodeId, Self::Node>>;

    /// Create a payload containing normal application data.
    #[since(version = "0.10.0")]
    fn normal(data: Self::D) -> Self {
        let payload = Self::blank();
        payload.with_normal(data)
    }

    /// Create a payload containing a membership.
    #[since(version = "0.10.0")]
    fn membership(membership: Membership<Self::NodeId, Self::Node>) -> Self {
        let payload = Self::blank();
        payload.with_membership(membership)
    }
}
