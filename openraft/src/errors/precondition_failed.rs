use display_more::DisplayOptionExt;
use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::type_config::alias::CommittedLeaderIdOf;
use crate::type_config::alias::LogIdOf;

/// A [`Precondition`] is not satisfied by the current Raft state.
///
/// [`Precondition`]: `crate::raft::Precondition`
#[since(version = "0.10.0")]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum PreconditionFailed<C>
where C: RaftTypeConfig
{
    /// The established leader is not the one [`Precondition::CommittedLeaderId`] required.
    ///
    /// `actual` is `None` when this node's vote is not committed, i.e. it has no established
    /// leader.
    ///
    /// [`Precondition::CommittedLeaderId`]: `crate::raft::Precondition::CommittedLeaderId`
    #[error("committed leader id mismatch: expected: {expected}, actual: {}", actual.display())]
    CommittedLeaderIdMismatch {
        /// The committed leader id that the precondition requires.
        expected: CommittedLeaderIdOf<C>,
        /// The committed leader id of this node's vote, or `None` if the vote is not committed.
        actual: Option<CommittedLeaderIdOf<C>>,
    },

    /// The last log id is not the one [`Precondition::LastLogId`] required.
    ///
    /// [`Precondition::LastLogId`]: `crate::raft::Precondition::LastLogId`
    #[error("last log id mismatch: expected: {}, actual: {}", expected.display(), actual.display())]
    LastLogIdMismatch {
        /// The last log id that the precondition requires; `None` means an empty log.
        expected: Option<LogIdOf<C>>,
        /// This node's last log id; `None` means an empty log.
        actual: Option<LogIdOf<C>>,
    },

    /// The effective membership is not the one [`Precondition::LastMembershipLogId`] required,
    /// meaning another membership change was proposed in between.
    ///
    /// [`Precondition::LastMembershipLogId`]: `crate::raft::Precondition::LastMembershipLogId`
    #[error("last membership log id mismatch: expected: {}, actual: {}", expected.display(), actual.display())]
    LastMembershipLogIdMismatch {
        /// The membership log id that the precondition requires; `None` means no membership log.
        expected: Option<LogIdOf<C>>,
        /// The log id of the effective membership; `None` means no membership log has been
        /// appended.
        actual: Option<LogIdOf<C>>,
    },
}
