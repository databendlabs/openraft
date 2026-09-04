use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::raft::ConflictHint;
use crate::type_config::alias::LogIdOf;

/// The follower's log does not match the leader's at the given index.
///
/// The follower rejects an `AppendEntries` request because `prev_log_id` from the
/// leader is not present in its local log.
#[since(version = "0.10.0", change = "added follower conflict hint")]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error("conflicting log-id: local={local:?} should be: {expect:?}; hint={hint:?}")]
pub struct ConflictingLogId<C>
where C: RaftTypeConfig
{
    pub expect: LogIdOf<C>,
    pub local: Option<LogIdOf<C>>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub hint: Option<ConflictHint<C>>,
}
