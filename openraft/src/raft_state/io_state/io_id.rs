use std::fmt;

use crate::raft_state::io_state::append_log_io_id::AppendLogIOId;
use crate::vote::CommittedVote;
use crate::ErrorSubject;
use crate::ErrorVerb;
use crate::LogId;
use crate::RaftTypeConfig;

/// An ID to uniquely identify an io operation to [`RaftLogStorage`].
///
/// [`RaftLogStorage`]: `crate::storage::RaftLogStorage`
pub(crate) enum IOId<C>
where C: RaftTypeConfig
{
    AppendLog(AppendLogIOId<C>),
}

impl<C> fmt::Display for IOId<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AppendLog(log_id) => write!(f, "AppendLog({})", log_id),
        }
    }
}

impl<C> IOId<C>
where C: RaftTypeConfig
{
    pub(crate) fn new_append_log(committed_vote: CommittedVote<C>, last_log_id: LogId<C::NodeId>) -> Self {
        Self::AppendLog(AppendLogIOId::new(committed_vote, last_log_id))
    }

    /// Return the `subject` of this io operation, such as `Log` or `Vote`.
    pub(crate) fn subject(&self) -> ErrorSubject<C> {
        match self {
            Self::AppendLog(log_id) => ErrorSubject::Log(log_id.log_id),
        }
    }

    /// Return the `verb` of this io operation, such as `Write` or `Read`.
    pub(crate) fn verb(&self) -> ErrorVerb {
        match self {
            Self::AppendLog(_) => ErrorVerb::Write,
        }
    }
}
