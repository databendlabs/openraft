use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::raft::AppendEntriesResponse;
use crate::raft::ConflictHint;

/// An optional probe response that preserves the existing AppendEntries response format.
#[since(version = "0.10.0")]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[non_exhaustive]
pub struct AppendEntriesResponseWithHint<C>
where C: RaftTypeConfig
{
    /// The ordinary AppendEntries result, with its existing durability guarantees.
    pub response: AppendEntriesResponse<C>,

    /// Search guidance returned on a conflict; it does not acknowledge replication.
    pub hint: Option<ConflictHint<C>>,
}

impl<C> AppendEntriesResponseWithHint<C>
where C: RaftTypeConfig
{
    /// Attach optional conflict guidance to an ordinary AppendEntries response.
    #[since(version = "0.10.0")]
    pub fn new(response: AppendEntriesResponse<C>, hint: Option<ConflictHint<C>>) -> Self {
        Self { response, hint }
    }
}
