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
    /// Selected follower log IDs, ordered by index and deduplicated.
    ///
    /// The local committed log ID comes first when known, followed by selected later leader
    /// boundaries. The last element is always the in-memory tail after truncation. An empty list
    /// means the follower has no log history, including no snapshot boundary.
    ///
    /// The first element may be uncommitted. The leader must compare every sample with its own
    /// log before using it as a common prefix. The list may omit leaders.
    pub log_ids: Vec<LogIdOf<C>>,
}

impl<C> ConflictHint<C>
where C: RaftTypeConfig
{
    /// Create a hint from ordered log IDs ending at the follower's tail.
    #[since(version = "0.10.0")]
    pub fn new(log_ids: Vec<LogIdOf<C>>) -> Self {
        Self { log_ids }
    }
}
