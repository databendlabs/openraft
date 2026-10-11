use std::fmt::Debug;
use std::fmt::Display;

use openraft_macros::since;

use crate::AppData;
use crate::base::OptionalFeatures;
use crate::membership::RaftMembership;

/// Defines operations for constructing and inspecting an entry payload.
#[since(
    version = "0.10.0",
    change = "replaced `NodeId` and `Node` associated types with `Membership`"
)]
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

    /// The membership type that a membership payload carries.
    #[since(version = "0.10.0")]
    type Membership: RaftMembership;

    /// Create a blank payload.
    ///
    /// OpenRaft uses a new blank payload as the base of a membership-change entry when the
    /// application does not supply a payload.
    #[since(version = "0.10.0")]
    fn blank() -> Self;

    /// Replace the normal application data in this payload.
    #[since(version = "0.10.0")]
    fn with_normal(self, data: Self::D) -> Self;

    /// Replace the membership in this payload.
    ///
    /// OpenRaft calls this method for every physical membership-change entry. An implementation
    /// may preserve other application fields.
    #[since(version = "0.10.0")]
    fn with_membership(self, membership: Self::Membership) -> Self;

    /// Return `Some(Membership)` if the entry payload contains a membership payload.
    #[since(version = "0.10.0", change = "use associated node types")]
    fn get_membership(&self) -> Option<Self::Membership>;

    /// Create a payload containing normal application data.
    #[since(version = "0.10.0")]
    fn normal(data: Self::D) -> Self {
        let payload = Self::blank();
        payload.with_normal(data)
    }

    /// Create a payload containing a membership.
    #[since(version = "0.10.0")]
    fn membership(membership: Self::Membership) -> Self {
        let payload = Self::blank();
        payload.with_membership(membership)
    }
}
