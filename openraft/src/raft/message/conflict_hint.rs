use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::type_config::alias::LogIdOf;

/// Follower log positions that can guide another AppendEntries probe.
///
/// These entries may not be flushed. A hint never acknowledges durable replication; the leader
/// must verify the chosen position with AppendEntries before counting it toward a quorum.
#[since(version = "0.10.0")]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[non_exhaustive]
pub struct ConflictHint<C>
where C: RaftTypeConfig
{
    /// The follower's in-memory tail after truncating the conflicting suffix.
    pub last_log_id: Option<LogIdOf<C>>,

    /// Retained log IDs, normally the last entry of recent leaders and a committed position.
    ///
    /// The list may omit leaders. Each position independently proves a common prefix when its
    /// committed leader ID also occurs in the leader's log.
    pub log_ids: Vec<LogIdOf<C>>,
}

impl<C> ConflictHint<C>
where C: RaftTypeConfig
{
    /// Create a hint from the follower's tail and selected retained log IDs.
    #[since(version = "0.10.0")]
    pub fn new(last_log_id: Option<LogIdOf<C>>, log_ids: Vec<LogIdOf<C>>) -> Self {
        Self { last_log_id, log_ids }
    }
}
