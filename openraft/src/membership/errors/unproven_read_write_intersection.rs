use openraft_macros::since;

/// OpenRaft cannot prove that every read quorum of a membership intersects every write quorum.
///
/// OpenRaft checks each membership that an application supplies with
/// [`intersects_with()`](quorum_set::QuorumIntersection::intersects_with), called on its read
/// quorum set against its write quorum set. It rejects the membership when the call returns
/// `Some(false)`. It also rejects it when the call returns `None`, unless
/// [`Config::allow_unproven_read_write_intersection`] is set.
///
/// [`Config::allow_unproven_read_write_intersection`]: crate::Config::allow_unproven_read_write_intersection
#[since(version = "0.10.0", change = "defined in membership::errors; errors re-exports it")]
#[since(version = "0.10.0")]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("cannot prove that every read quorum intersects every write quorum: {membership}")]
pub struct UnprovenReadWriteIntersection {
    /// The rejected membership, formatted with `Display`.
    pub membership: String,
}
