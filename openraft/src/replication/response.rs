use std::fmt;

use crate::RaftTypeConfig;
use crate::display_ext::DisplayOptionExt;
use crate::display_ext::DisplayResultExt;
use crate::replication::ReplicationSessionId;
use crate::type_config::alias::LogIdOf;

/// The response of replication command.
///
/// Update the `matched` log id of a replication target.
/// Sent by a replication task `ReplicationCore`.
#[derive(Debug)]
pub(crate) struct Progress<C>
where C: RaftTypeConfig
{
    /// The ID of the target node for which the match index is to be updated.
    pub(crate) target: C::NodeId,

    /// The request by this leader has been successfully handled by the target node
    /// or an error in string.
    ///
    /// A successful result can still be log matching or log conflicting.
    /// In either case, the request is considered accepted, i.e., this leader is still valid to
    /// the target node.
    ///
    /// The result also tracks the time when this request is sent.
    pub(crate) result: Result<ReplicationResult<C>, String>,

    /// In which session this message is sent.
    ///
    /// This session id identifies a certain leader(by vote) that is replicating to a certain
    /// group of nodes.
    ///
    /// A message should be discarded if it does not match the present vote and
    /// membership_log_id.
    pub(crate) session_id: ReplicationSessionId<C>,
}

impl<C> fmt::Display for Progress<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "replication::Progress: target={}, result: {}, session_id: {}",
            self.target,
            self.result.display(),
            self.session_id
        )
    }
}

/// Result of an append-entries replication
///
/// Ok for matching, Err for conflict.
#[derive(Clone, Debug)]
pub(crate) struct ReplicationResult<C: RaftTypeConfig>(pub(crate) Result<Option<LogIdOf<C>>, LogIdOf<C>>);

impl<C> fmt::Display for ReplicationResult<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Ok(matching) => write!(f, "(Match:{})", matching.display()),
            Err(conflict) => write!(f, "(Conflict:{})", conflict),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::engine::testing::UTConfig;
    use crate::engine::testing::log_id;
    use crate::replication::response::ReplicationResult;

    #[test]
    fn test_replication_result_display() {
        let result = ReplicationResult::<UTConfig>(Ok(Some(log_id(1, 2, 3))));
        let want = format!("(Match:{})", log_id(1, 2, 3));
        assert!(result.to_string().ends_with(&want), "{}", result.to_string());

        let result = ReplicationResult::<UTConfig>(Err(log_id(1, 2, 3)));
        let want = format!("(Conflict:{})", log_id(1, 2, 3));
        assert!(result.to_string().ends_with(&want), "{}", result.to_string());
    }
}
