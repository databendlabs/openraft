use openraft_macros::since;

/// OpenRaft cannot prove that every read quorum of a membership intersects every write quorum.
///
/// OpenRaft checks each membership that an application supplies with
/// [`RaftMembership::read_write_quorums_intersect()`]. It rejects the membership unless the call
/// returns `Some(true)`.
///
/// [`RaftMembership::read_write_quorums_intersect()`]: crate::membership::RaftMembership::read_write_quorums_intersect
#[since(version = "0.10.0")]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("cannot prove that every read quorum intersects every write quorum: {membership}")]
pub struct UnprovenReadWriteIntersection {
    /// The rejected membership, formatted with `Display`.
    pub membership: String,
}
