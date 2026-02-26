use crate::RaftTypeConfig;
use crate::raft::AppendEntriesResponse;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::VoteOf;

/// Reason a node refuses to acknowledge another node's leadership claim.
///
/// This error is returned when a node rejects a vote request or an
/// `AppendEntries` RPC because the requesting node is not qualified to be
/// leader from this node's perspective.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RejectLeadership<C: RaftTypeConfig> {
    /// The node has already granted a vote to — or seen a committed vote from —
    /// a candidate with an equal or higher term, so it refuses this one.
    #[error("reject leadership: local vote {0} is not less than the leader's")]
    ByVote(VoteOf<C>),

    /// The node's log is at least as up-to-date as the candidate's, so granting
    /// the vote would risk data loss (standard Raft log-completeness check).
    #[allow(dead_code)]
    #[error("reject leadership: local last-log-id {0:?} is not less than the leader's")]
    ByLastLogId(Option<LogIdOf<C>>),
}

impl<C> From<RejectLeadership<C>> for AppendEntriesResponse<C>
where C: RaftTypeConfig
{
    fn from(r: RejectLeadership<C>) -> Self {
        match r {
            RejectLeadership::ByVote(v) => AppendEntriesResponse::HigherVote(v),
            RejectLeadership::ByLastLogId(_) => {
                unreachable!("the leader should always has a greater last log id")
            }
        }
    }
}
