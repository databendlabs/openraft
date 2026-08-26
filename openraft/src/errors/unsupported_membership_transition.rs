use std::collections::BTreeSet;

use openraft_macros::since;

use crate::node::NodeId;

/// The proposed membership is not one of the transitions a direct membership append supports.
///
/// A direct append writes the caller's exact membership as one log entry, without an intermediate
/// joint membership. It is accepted only when quorum intersection can be proved by a simple rule:
/// two uniform memberships whose voter sets differ by at most one node id, or two memberships that
/// share an exactly equal voter set.
///
/// The rule is conservative, so a rejected transition is **unsupported**, not necessarily unsafe.
/// Some rejected transitions do have intersecting quorums; Openraft does not run a general
/// quorum-intersection solver to find them.
#[since(version = "0.10.0")]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("unsupported membership transition: from voters {previous:?} to voters {proposed:?}")]
pub struct UnsupportedMembershipTransition<NID>
where NID: NodeId
{
    /// The voter sets of the last effective membership.
    pub previous: Vec<BTreeSet<NID>>,

    /// The voter sets of the membership the caller proposed.
    pub proposed: Vec<BTreeSet<NID>>,
}
