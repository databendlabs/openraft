use openraft_macros::since;

use crate::errors::ChangeMembershipError;
use crate::errors::EmptyMembership;
use crate::errors::LearnerNotFound;
use crate::errors::NodeNotFound;
use crate::node::NodeId;
use crate::vote::RaftCommittedLeaderId;

/// Errors occur when building a [`Membership`].
///
/// [`Membership`]: crate::membership::Membership
#[since(version = "0.10.0", change = "removed `C: RaftTypeConfig` generic parameter")]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum MembershipError<NID>
where NID: NodeId
{
    /// The membership configuration is empty.
    #[error(transparent)]
    EmptyMembership(#[from] EmptyMembership),

    /// A required node was not found.
    #[error(transparent)]
    NodeNotFound(#[from] NodeNotFound<NID>),
}

/// Translate a membership validation failure into a rejected change request.
///
/// The two enums overlap, but they describe different layers: [`MembershipError`] says a
/// membership value is invalid, while [`ChangeMembershipError`] says a caller's request was
/// refused. Embedding one in the other would collapse that distinction and, worse, lose the
/// specific advice below.
///
/// `NodeNotFound` deliberately becomes `LearnerNotFound`. The only way membership validation
/// reports a missing node is a voter with no `Node` entry, which means the caller tried to promote
/// a node it never added. `LearnerNotFound` says exactly that, whereas `NodeNotFound` would only
/// say the node is missing. The `operation` field dropped in the process is always
/// `Operation::None` on this path, so nothing diagnostic is lost.
impl<CLID, NID> From<MembershipError<NID>> for ChangeMembershipError<CLID, NID>
where
    CLID: RaftCommittedLeaderId,
    NID: NodeId,
{
    fn from(me: MembershipError<NID>) -> Self {
        match me {
            MembershipError::EmptyMembership(e) => ChangeMembershipError::EmptyMembership(e),
            MembershipError::NodeNotFound(e) => {
                ChangeMembershipError::LearnerNotFound(LearnerNotFound { node_id: e.node_id })
            }
        }
    }
}
