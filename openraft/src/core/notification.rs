use std::fmt;

use crate::core::sm;
use crate::display_ext::DisplayInstantExt;
use crate::raft::VoteResponse;
use crate::raft_state::IOId;
use crate::replication;
use crate::replication::ReplicationSessionId;
use crate::type_config::alias::InstantOf;
use crate::vote::CommittedVote;
use crate::vote::NonCommittedVote;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::Vote;

/// A message coming from the internal components.
pub(crate) enum Notification<C>
where C: RaftTypeConfig
{
    VoteResponse {
        target: C::NodeId,
        resp: VoteResponse<C>,

        /// The candidate that sent the vote request.
        ///
        /// A vote identifies a unique server state.
        candidate_vote: NonCommittedVote<C>,
    },

    /// A Leader sees a higher `vote` when replicating.
    HigherVote {
        /// The ID of the target node from which the new term was observed.
        target: C::NodeId,

        /// The higher vote observed.
        higher: Vote<C>,

        /// The Leader that sent replication request.
        leader_vote: CommittedVote<C>,
        // TODO: need this?
        // /// The cluster this replication works for.
        // membership_log_id: Option<LogId<C>>,
    },

    /// [`StorageError`] error has taken place locally(not on remote node),
    /// and [`RaftCore`](`crate::core::RaftCore`) needs to shutdown.
    StorageError { error: StorageError<C> },

    /// Completion of an IO operation to local store.
    LocalIO { io_id: IOId<C> },

    /// Result of executing a command sent from network worker.
    ReplicationProgress { progress: replication::Progress<C> },

    HeartbeatProgress {
        session_id: ReplicationSessionId<C>,
        sending_time: InstantOf<C>,
        target: C::NodeId,
    },

    /// Result of executing a command sent from state machine worker.
    StateMachine { command_result: sm::CommandResult<C> },

    /// A tick event to wake up RaftCore to check timeout etc.
    Tick {
        /// ith tick
        i: u64,
    },
}

impl<C> Notification<C>
where C: RaftTypeConfig
{
    pub(crate) fn sm(command_result: sm::CommandResult<C>) -> Self {
        Self::StateMachine { command_result }
    }
}

impl<C> fmt::Display for Notification<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VoteResponse {
                target,
                resp,
                candidate_vote,
            } => {
                write!(
                    f,
                    "VoteResponse: from target={}, to candidate_vote: {}, {}",
                    target, candidate_vote, resp
                )
            }
            Self::HigherVote {
                ref target,
                higher: ref new_vote,
                leader_vote: ref vote,
            } => {
                write!(
                    f,
                    "Seen a higher vote: target: {}, vote: {}, server_state_vote: {}",
                    target, new_vote, vote
                )
            }
            Self::StorageError { error } => write!(f, "StorageError: {}", error),
            Self::LocalIO { io_id } => write!(f, "IOFlushed: {}", io_id),
            Self::ReplicationProgress { progress } => {
                write!(f, "{}", progress)
            }
            Self::HeartbeatProgress {
                session_id: leader_vote,
                sending_time,
                target,
            } => {
                write!(
                    f,
                    "HeartbeatProgress: target={}, leader_vote: {}, sending_time: {}",
                    target,
                    leader_vote,
                    sending_time.display(),
                )
            }
            Self::StateMachine { command_result } => {
                write!(f, "{}", command_result)
            }
            Self::Tick { i } => {
                write!(f, "Tick {}", i)
            }
        }
    }
}
