use crate::RaftTypeConfig;
use crate::type_config::alias::LogIdOf;

/// The follower's log does not match the leader's at the given index.
///
/// The follower rejects an `AppendEntries` request because `prev_log_id` from the
/// leader is not present in its local log.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("conflicting log-id: local={local:?} should be: {expect:?}")]
pub struct ConflictingLogId<C: RaftTypeConfig> {
    /// The log id that the leader expects at this index: the `prev_log_id` of its request.
    pub expect: LogIdOf<C>,
    /// The follower's log id at the same index, or `None` if it has no entry there or the
    /// transport did not report it.
    pub local: Option<LogIdOf<C>>,
}
