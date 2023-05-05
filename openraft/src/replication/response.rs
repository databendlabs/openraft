use crate::replication::ReplicationResult;
use crate::replication::ReplicationSessionId;
use crate::MessageSummary;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::Vote;

/// The response of replication command.
#[derive(Debug)]
pub(crate) enum Response<C>
where C: RaftTypeConfig
{
    // /// Logs that are submitted to append has been persisted to disk.
    // LogPersisted {},
    /// Update the `matched` log id of a replication target.
    /// Sent by a replication task `ReplicationCore`.
    Progress {
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
    /// [`RaftCore`](`crate::core::RaftCore`) needs to shutdown. Sent by a replication task
    /// [`crate::replication::ReplicationCore`].
    StorageError { error: StorageError<C::NodeId> },

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
}

impl<C> MessageSummary<Response<C>> for Response<C>
where C: RaftTypeConfig
{
    fn summary(&self) -> String {
        match self {
            Self::Progress {
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

            Self::StorageError { error } => format!("ReplicationStorageError: {}", error),

            Self::HigherVote {
                ref target,
                higher: ref new_vote,
                ref vote,
            } => {
                format!(
                    "Seen a higher vote: target: {}, vote: {}, server_state_vote: {}",
                    target, new_vote, vote
                )
            }
        }
    }
}
