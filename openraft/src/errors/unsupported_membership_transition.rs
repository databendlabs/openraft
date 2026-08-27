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
///
/// # Rejected transitions
///
/// Replacing one voter with another in one step:
///
/// ```text
/// [{a,b,c}] -> [{b,c,d}]
/// ```
///
/// The two uniform voter sets differ by two node ids, `a` and `d`. Quorum `{a,b}` of the previous
/// membership and quorum `{c,d}` of the proposed one do not intersect, so the transition is
/// unsafe. Append `[{a,b,c,d}]` first, then `[{b,c,d}]`.
///
/// Adding two voters in one step:
///
/// ```text
/// [{a,b,c}] -> [{a,b,c,x,y}]
/// ```
///
/// Quorum `{a,b}` of the previous membership and quorum `{c,x,y}` of the proposed one do not
/// intersect. Append `x` and `y` one at a time.
///
/// Overlapping, but not exactly equal, voter sets:
///
/// ```text
/// [{a,b,c}, {a,b,d}] -> [{a,b,e}]
/// ```
///
/// No previous voter set equals `{a,b,e}`. Sharing `a` and `b` does not help: quorum `{a,c,d}` of
/// the previous membership and quorum `{b,e}` of the proposed one do not intersect.
///
/// A safe transition this rule still rejects:
///
/// ```text
/// [{a,b,c,d}, {a,b,c,e}] -> [{a,b,d,e}, {a,c,d,e}]
/// ```
///
/// Every voter set holds four of the five node ids `{a,b,c,d,e}`, so every quorum holds at least
/// three of them, and any two such quorums intersect. No voter set is equal across the two
/// memberships, so this rule cannot see that and rejects the transition.
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
