use crate::replication::ReplicationSessionId;
use crate::utime::UTime;
use crate::AsyncRuntime;
use crate::LogId;
use crate::MessageSummary;
use crate::NodeId;
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
        request_id: u64,

        /// The request by this leader has been successfully handled by the target node,
        /// or an error in string.
        ///
        /// A successful result can still be log matching or log conflicting.
        /// In either case, the request is considered accepted, i.e., this leader is still valid to
        /// the target node.
        ///
        /// The result also track the time when this request is sent.
        result: Result<UTime<ReplicationResult<C::NodeId>, <C::AsyncRuntime as AsyncRuntime>::Instant>, String>,

        /// In which session this message is sent.
        ///
        /// This session id identifies a certain leader(by vote) that is replicating to a certain
        /// group of nodes.
        ///
        /// A message should be discarded if it does not match the present vote and
        /// membership_log_id.
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
                request_id: ref id,
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

/// Result of an replication action.
#[derive(Clone, Debug)]
pub(crate) enum ReplicationResult<NID: NodeId> {
    Matching(Option<LogId<NID>>),
    Conflict(LogId<NID>),
}
