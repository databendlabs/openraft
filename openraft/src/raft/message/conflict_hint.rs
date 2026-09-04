use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::type_config::alias::LogIdOf;

/// Follower log bounds returned with an AppendEntries conflict.
#[since(version = "0.10.0", change = "added follower conflict recovery bounds")]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct ConflictHint<C>
where C: RaftTypeConfig
{
    /// The follower's last log ID before it handled the conflicting request.
    pub last_log_id: Option<LogIdOf<C>>,

    /// The last log ID known to be locally committed on the follower.
    #[cfg_attr(feature = "serde", serde(default))]
    pub committed_log_id: Option<LogIdOf<C>>,
}

impl<C> ConflictHint<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(last_log_id: Option<LogIdOf<C>>, committed_log_id: Option<LogIdOf<C>>) -> Self {
        Self {
            last_log_id,
            committed_log_id,
        }
    }
}
