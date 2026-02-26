use crate::RaftTypeConfig;
use crate::type_config::alias::LogIdOf;

/// The follower's log does not match the leader's at the given index.
///
/// The follower rejects an `AppendEntries` request because `prev_log_id` from the
/// leader is not present in its local log.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("conflicting log-id: local={local:?} should be: {expect:?}")]
pub struct ConflictingLogId<C: RaftTypeConfig> {
    pub expect: LogIdOf<C>,
    pub local: Option<LogIdOf<C>>,
}
