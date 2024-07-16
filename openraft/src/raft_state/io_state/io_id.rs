use std::cmp::Ordering;
use std::fmt;
use std::ops::Deref;

use crate::raft_state::io_state::log_io_id::LogIOId;
use crate::vote::CommittedVote;
use crate::vote::NonCommittedVote;
use crate::ErrorSubject;
use crate::ErrorVerb;
use crate::LogId;
use crate::RaftTypeConfig;
use crate::Vote;

/// An ID to uniquely identify an monotonic increasing io operation to [`RaftLogStorage`].
///
/// Not all IO to [`RaftLogStorage`] are included by this struct:
/// only the IOs make progress in the raft log are included.
/// Including [`save_vote()`] and [`append()`] logs.
///
/// [`purge()`] and [`truncate()`] are not included,
/// because [`truncate()`] just remove junks: the logs that are conflict with the leader.
/// And [`purge()`] just remove logs that are already committed.
///
/// [`RaftLogStorage`]: `crate::storage::RaftLogStorage`
/// [`save_vote()`]: `crate::storage::RaftLogStorage::save_vote()`
/// [`append()`]: `crate::storage::RaftLogStorage::append()`
/// [`truncate()`]: `crate::storage::RaftLogStorage::truncate()`
/// [`purge()`]: `crate::storage::RaftLogStorage::purge()`
#[derive(Debug, Clone, Copy)]
#[derive(PartialEq, Eq)]
pub(crate) enum IOId<C>
where C: RaftTypeConfig
{
    /// Saving a non-committed vote, this kind of IO is not related to any log entries.
    Vote(NonCommittedVote<C>),

    /// Saving log entries by a Leader, which is identified by a committed vote.
    Log(LogIOId<C>),
}

impl<C> fmt::Display for IOId<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Vote(vote) => write!(f, "({})", vote),
            Self::Log(log_id) => write!(f, "({})", log_id),
        }
    }
}

/// Implement `PartialOrd` for `IOId`
///
/// Compare the `vote` first, if votes are equal, compare the `last_log_id`.
impl<C> PartialOrd for IOId<C>
where C: RaftTypeConfig
{
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        let res = self.vote_ref().partial_cmp(other.vote_ref())?;
        match res {
            Ordering::Less => Some(Ordering::Less),
            Ordering::Greater => Some(Ordering::Greater),
            Ordering::Equal => self.last_log_id().partial_cmp(&other.last_log_id()),
        }
    }
}

impl<C> IOId<C>
where C: RaftTypeConfig
{
    pub(crate) fn new(vote: Vote<C::NodeId>) -> Self {
        if vote.is_committed() {
            Self::new_log_io(vote.into_committed(), None)
        } else {
            Self::new_vote_io(vote.into_non_committed())
        }
    }

    pub(crate) fn new_vote_io(vote: NonCommittedVote<C>) -> Self {
        Self::Vote(vote)
    }

    pub(crate) fn new_log_io(committed_vote: CommittedVote<C>, last_log_id: Option<LogId<C::NodeId>>) -> Self {
        Self::Log(LogIOId::new(committed_vote, last_log_id))
    }

    pub(crate) fn vote_ref(&self) -> &Vote<C::NodeId> {
        match self {
            Self::Vote(vote) => vote.deref(),
            Self::Log(log_io_id) => log_io_id.committed_vote.deref(),
        }
    }
    pub(crate) fn last_log_id(&self) -> Option<&LogId<C::NodeId>> {
        match self {
            Self::Vote(_) => None,
            Self::Log(log_io_id) => log_io_id.log_id.as_ref(),
        }
    }

    /// Return the `subject` of this io operation, such as `Log` or `Vote`.
    pub(crate) fn subject(&self) -> ErrorSubject<C> {
        match self {
            Self::Vote(_vote) => ErrorSubject::Vote,
            Self::Log(log_io_id) => {
                if let Some(log_id) = log_io_id.log_id {
                    ErrorSubject::Log(log_id)
                } else {
                    ErrorSubject::Logs
                }
            }
        }
    }

    /// Return the `verb` of this io operation, such as `Write` or `Read`.
    pub(crate) fn verb(&self) -> ErrorVerb {
        ErrorVerb::Write
    }
}
