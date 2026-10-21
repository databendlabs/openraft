use std::collections::BTreeSet;

use openraft_macros::since;

use crate::node::NodeId;

/// The proposed membership is not one of the transitions a direct membership append supports.
///
/// A direct append writes the caller's exact membership as one log entry, without an intermediate
/// joint membership. It is accepted only when quorum intersection can be proved by a simple rule:
/// a voter set of one membership and a voter set of the other have intersecting majorities, i.e.
/// every majority of the first intersects every majority of the second.
///
/// For two uniform memberships the rule is exact, so a rejected transition between them is unsafe.
/// When either membership has more than one voter set, the rule is conservative, so a rejected
/// transition is **unsupported**, not necessarily unsafe. Some rejected transitions do have
/// intersecting quorums; Openraft does not run a general quorum-intersection solver to find them.
///
/// # Rejected transitions
///
/// Replacing one of three voters with another in one step:
///
/// ```text
/// [{a,b,c}] -> [{b,c,d}]
/// ```
///
/// Quorum `{a,b}` of the previous membership and quorum `{c,d}` of the proposed one do not
/// intersect, so the transition is unsafe. Append `[{a,b,c,d}]` first, then `[{b,c,d}]`.
///
/// Adding two voters to three in one step:
///
/// ```text
/// [{a,b,c}] -> [{a,b,c,x,y}]
/// ```
///
/// Quorum `{a,b}` of the previous membership and quorum `{c,x,y}` of the proposed one do not
/// intersect. Append `x` and `y` one at a time.
///
/// Voter sets that overlap too little:
///
/// ```text
/// [{a,b,c}, {a,b,d}] -> [{a,b,e}]
/// ```
///
/// No previous voter set has majorities that intersect those of `{a,b,e}`: majority `{a,c}` of
/// `{a,b,c}` and majority `{a,d}` of `{a,b,d}` each miss majority `{b,e}`. Sharing `a` and `b` does
/// not help: quorum `{a,c,d}` of the previous membership and quorum `{b,e}` of the proposed one do
/// not intersect.
///
/// A safe transition this rule still rejects:
///
/// ```text
/// [{a,b,c}, {a,d,e}] -> [{b,c,d,e}]
/// ```
///
/// Every previous quorum holds at least two of `{a,b,c}` and at least two of `{a,d,e}`, and only
/// `a` is in both, so it holds at least two of `{b,c,d,e}`. Every proposed quorum holds at least
/// three of these four node ids, so any two such quorums intersect. No voter set pair shows it:
/// majority `{a,b}` of `{a,b,c}` misses majority `{c,d,e}` of `{b,c,d,e}`, and majority `{a,d}`
/// of `{a,d,e}` misses `{b,c,e}`. So this rule cannot see the intersection and rejects the
/// transition.
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
