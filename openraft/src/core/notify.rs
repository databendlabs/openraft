use crate::core::sm;
use crate::raft::VoteResponse;
use crate::replication::ReplicationResult;
use crate::replication::ReplicationSessionId;
use crate::MessageSummary;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::Vote;

/// A message coming from the internal components.
pub(crate) enum Notify<C>
where C: RaftTypeConfig
{
    VoteResponse {
        target: C::NodeId,
        resp: VoteResponse<C::NodeId>,

        /// Which ServerState sent this message. It is also the requested vote.
        vote: Vote<C::NodeId>,
    },

    /// A tick event to wake up RaftCore to check timeout etc.
    Tick {
        /// ith tick
        i: u64,
    },

    // /// Logs that are submitted to append has been persisted to disk.
    // LogPersisted {},
    /// Update the `matched` log id of a replication target.
    /// Sent by a replication task `ReplicationCore`.
    UpdateReplicationProgress {
        /// The ID of the target node for which the match index is to be updated.
        target: C::NodeId,

        /// The id of the subject that submit this replication action.
        ///
        /// It is only used for debugging purpose.
        id: u64,

        /// Either the last log id that has been successfully replicated to the target,
        /// or an error in string.
        result: Result<ReplicationResult<C::NodeId>, String>,

        /// In which session this message is sent.
        /// A replication session(vote,membership_log_id) should ignore message from other session.
        session_id: ReplicationSessionId<C::NodeId>,
    },

    /// [`StorageError`] error has taken place locally(not on remote node) when replicating, and
    /// [`RaftCore`] needs to shutdown. Sent by a replication task
    /// [`crate::replication::ReplicationCore`].
    ReplicationStorageError { error: StorageError<C::NodeId> },

    /// ReplicationCore has seen a higher `vote`.
    /// Sent by a replication task `ReplicationCore`.
    HigherVote {
        /// The ID of the target node from which the new term was observed.
        target: C::NodeId,

        /// The higher vote observed.
        higher: Vote<C::NodeId>,

        /// Which ServerState sent this message
        vote: Vote<C::NodeId>,
        // TODO: need this?
        // /// The cluster this replication works for.
        // membership_log_id: Option<LogId<C::NodeId>>,
    },

    /// Result of executing a command sent from state machine worker.
    StateMachine { command_result: sm::CommandResult<C> },
}

impl<C> MessageSummary<Notify<C>> for Notify<C>
where C: RaftTypeConfig
{
    fn summary(&self) -> String {
        match self {
            Notify::VoteResponse { target, resp, vote } => {
                format!("VoteResponse: from: {}: {}, res-vote: {}", target, resp.summary(), vote)
            }
            Notify::Tick { i } => {
                format!("Tick {}", i)
            }
            Notify::UpdateReplicationProgress {
                ref target,
                ref id,
                ref result,
                ref session_id,
            } => {
                format!(
                    "UpdateReplicationProgress: target: {}, id: {}, result: {:?}, session_id: {}",
                    target, id, result, session_id,
                )
            }
            Notify::HigherVote {
                ref target,
                higher: ref new_vote,
                ref vote,
            } => {
                format!(
                    "Seen a higher vote: target: {}, vote: {}, server_state_vote: {}",
                    target, new_vote, vote
                )
            }
            Notify::ReplicationStorageError { error } => format!("ReplicationFatal: {}", error),
            Notify::StateMachine { command_result: done } => {
                format!("StateMachine command done: {:?}", done)
            }
        }
    }
}
