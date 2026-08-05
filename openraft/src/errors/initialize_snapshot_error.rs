use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::errors::NotAllowed;
use crate::errors::NotInMembers;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::VoteOf;

/// An error returned when initializing a pristine node from a snapshot.
#[since(version = "0.10.0", change = "added snapshot initialization")]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum InitializeSnapshotError<C>
where C: RaftTypeConfig
{
    /// Initialization is not allowed in the current state.
    #[error(transparent)]
    NotAllowed(#[from] NotAllowed<C>),

    /// This node is not included in the snapshot membership.
    #[error(transparent)]
    NotInMembers(#[from] NotInMembers<C>),

    /// The snapshot has no applied log position.
    #[error("the initialization snapshot has no last log ID")]
    MissingLastLogId,

    /// The supplied vote is committed and would establish a leader before recovery completes.
    #[error("the snapshot initialization vote must be uncommitted: {vote}")]
    CommittedVote {
        /// The invalid committed vote.
        vote: VoteOf<C>,
    },

    /// The supplied vote names a node other than the node being initialized.
    #[error("the snapshot initialization vote {vote} must name local node {node_id}")]
    VoteForAnotherNode {
        /// The local node ID.
        node_id: C::NodeId,
        /// The invalid vote.
        vote: VoteOf<C>,
    },

    /// The supplied vote is not above the snapshot's last log leader.
    #[error("the snapshot initialization vote {vote} must be above last log ID {last_log_id}")]
    VoteNotAboveSnapshot {
        /// The vote that would establish the recovery term floor.
        vote: VoteOf<C>,
        /// The snapshot's last applied log ID.
        last_log_id: LogIdOf<C>,
    },

    /// The supplied vote could not advance the pristine node's persisted vote.
    #[error("the snapshot initialization vote {vote} was rejected by local vote {current_vote}")]
    VoteRejected {
        /// The rejected recovery vote.
        vote: VoteOf<C>,
        /// The local persisted vote.
        current_vote: VoteOf<C>,
    },
}
