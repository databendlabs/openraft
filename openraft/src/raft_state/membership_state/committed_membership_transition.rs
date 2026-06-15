use crate::log_id::LogId;
use crate::vote::RaftCommittedLeaderId;

/// The change of the committed membership config's log id.
///
/// Returned by [`MembershipState::commit`](super::MembershipState::commit) when committing
/// advances the committed membership config, carrying the committed membership log id before and
/// after the update.
#[derive(Debug)]
#[derive(PartialEq, Eq)]
pub(crate) struct CommittedMembershipTransition<CLID>
where CLID: RaftCommittedLeaderId
{
    /// The committed membership log id before the update; `None` if no membership was committed
    /// yet.
    pub(crate) before: Option<LogId<CLID>>,

    /// The committed membership log id after the update.
    pub(crate) after: LogId<CLID>,
}
