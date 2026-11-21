use openraft_macros::since;

/// The current membership does not support appending the proposed membership in one log entry.
///
/// The configured [`RaftMembership::supports_direct_transition_to()`] decides whether the append
/// can proceed without an intermediate joint membership. It may return `true` only if every read
/// quorum of either membership intersects every write quorum of the other. A `false` result means
/// the implementation does not support the direct append; it does not necessarily mean the
/// transition is unsafe.
///
/// # Default majority membership
///
/// The default [`Membership`](crate::Membership) supports the direct transition when a voter set of
/// one membership and a voter set of the other have intersecting majorities: every majority of the
/// first intersects every majority of the second. This rule is exact when both memberships are
/// uniform (one voter set each), so a rejected transition between them is unsafe. With more than
/// one voter set in either membership, the rule can reject transitions that are safe.
///
/// ## Rejected transitions
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
///
/// [`RaftMembership::supports_direct_transition_to()`]: crate::membership::RaftMembership::supports_direct_transition_to
#[since(
    version = "0.10.0",
    change = "removed `NID` generic parameter; `previous` and `proposed` from `Vec<BTreeSet<NID>>` to `String`"
)]
#[since(version = "0.10.0")]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("unsupported membership transition: from {previous} to {proposed}")]
pub struct UnsupportedMembershipTransition {
    /// The last effective membership, formatted with `Display`.
    pub previous: String,

    /// The membership the caller proposed, formatted with `Display`.
    pub proposed: String,
}
