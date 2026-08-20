use std::fmt;

use display_more::DisplayOptionExt;
use openraft_macros::since;

use crate::RaftState;
use crate::RaftTypeConfig;
use crate::errors::PreconditionFailed;
use crate::raft_state::LogStateReader;
use crate::type_config::alias::CommittedLeaderIdOf;
use crate::type_config::alias::LogIdOf;
use crate::vote::raft_vote::RaftVoteExt;

/// A condition on the Raft state that must hold before an operation is proposed.
///
/// The Raft core checks it on the leader, against the state at the moment the
/// operation is about to be proposed. If it does not hold, nothing is written
/// and the operation fails with [`PreconditionFailed`].
#[since(version = "0.10.0")]
#[derive(Debug, Clone)]
#[derive(PartialEq, Eq)]
pub enum Precondition<C>
where C: RaftTypeConfig
{
    /// Require `committed_leader_id` to still be the established leader.
    ///
    /// Satisfied only when this node's vote is committed and elects exactly `committed_leader_id`.
    /// A node whose vote is not yet committed has no established leader and never satisfies it.
    CommittedLeaderId {
        committed_leader_id: CommittedLeaderIdOf<C>,
    },

    /// Require the last log id to be exactly `last_log_id`, where `None` means an empty log.
    ///
    /// The last log entry may be uncommitted, and an uncommitted tail is reverted when a new
    /// leader is elected without having seen it. Satisfying this condition therefore does not
    /// guarantee the matched entry survives.
    LastLogId { last_log_id: Option<LogIdOf<C>> },

    /// Require the effective membership to be the one stored at `last_membership_log_id`, where
    /// `None` means no membership log has been appended.
    ///
    /// Use it to serialize membership changes: the change is proposed only while the effective
    /// membership is still the one the caller based its decision on, so a concurrent change made
    /// in between is rejected instead of silently overwritten.
    ///
    /// The effective membership may itself be uncommitted, and an uncommitted membership log is
    /// reverted when a new leader is elected without having seen it. Satisfying this condition
    /// therefore does not guarantee the matched membership survives.
    LastMembershipLogId { last_membership_log_id: Option<LogIdOf<C>> },
}

impl<C> Precondition<C>
where C: RaftTypeConfig
{
    /// Return `Ok(())` if `state` satisfies this condition, otherwise the mismatch it found.
    pub fn ensure_satisfied(&self, state: &RaftState<C>) -> Result<(), PreconditionFailed<C>> {
        match self {
            Precondition::CommittedLeaderId { committed_leader_id } => {
                let actual = state.vote_ref().try_to_committed_leader_id();
                if actual.as_ref() == Some(committed_leader_id) {
                    Ok(())
                } else {
                    Err(PreconditionFailed::CommittedLeaderIdMismatch {
                        expected: committed_leader_id.clone(),
                        actual,
                    })
                }
            }
            Precondition::LastLogId { last_log_id } => {
                let actual = state.last_log_id();
                if actual == last_log_id.as_ref() {
                    Ok(())
                } else {
                    Err(PreconditionFailed::LastLogIdMismatch {
                        expected: last_log_id.clone(),
                        actual: actual.cloned(),
                    })
                }
            }
            Precondition::LastMembershipLogId { last_membership_log_id } => {
                let actual = state.membership_state.effective().log_id();
                if actual == last_membership_log_id {
                    Ok(())
                } else {
                    Err(PreconditionFailed::LastMembershipLogIdMismatch {
                        expected: last_membership_log_id.clone(),
                        actual: actual.clone(),
                    })
                }
            }
        }
    }
}

impl<C> fmt::Display for Precondition<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Precondition::CommittedLeaderId { committed_leader_id } => {
                write!(f, "CommittedLeaderId({})", committed_leader_id)
            }
            Precondition::LastLogId { last_log_id } => {
                write!(f, "LastLogId({})", last_log_id.display())
            }
            Precondition::LastMembershipLogId { last_membership_log_id } => {
                write!(f, "LastMembershipLogId({})", last_membership_log_id.display())
            }
        }
    }
}

#[cfg(test)]
mod precondition_test;
