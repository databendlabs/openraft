use std::fmt;

use display_more::DisplayOptionExt;

use crate::RaftTypeConfig;
use crate::StorageError;
use crate::core::NotificationName;
use crate::core::sm;
use crate::progress::inflight_id::InflightId;
use crate::progress::stream_id::StreamId;
use crate::proposer::SendStamp;
use crate::raft::VoteResponse;
use crate::raft_state::IOId;
use crate::replication;
use crate::type_config::alias::CommittedVoteOf;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::UncommittedVoteOf;
use crate::type_config::alias::VoteOf;

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
        candidate_vote: UncommittedVoteOf<C>,
    },

    /// Response to a Pre-Vote request sent by a pre-candidate.
    ///
    /// Handling the Pre-Vote request changes no persistent state, but handling this response can.
    /// A response to the current pre-vote round goes to the engine: a grant advances the tally, and
    /// a quorum of grants starts a real election, which saves a new vote. A rejection that carries
    /// a higher vote makes this node adopt that vote and save it as a non-committed vote. A
    /// response to an earlier round is ignored. See the [Pre-Vote protocol].
    ///
    /// [Pre-Vote protocol]: crate::docs::protocol::pre_vote
    PreVoteResponse {
        target: C::NodeId,
        resp: VoteResponse<C>,

        /// The pre-candidate vote (the hypothetical next-term vote) this response answers.
        candidate_vote: UncommittedVoteOf<C>,
    },

    /// A Leader sees a higher `vote` when replicating.
    HigherVote {
        /// The ID of the target node from which the new term was observed.
        target: C::NodeId,

        /// The higher vote observed.
        higher: VoteOf<C>,

        /// The Leader that sent the replication request.
        leader_vote: CommittedVoteOf<C>,
    },

    /// [`StorageError`] error has taken place locally(not on remote node),
    /// and [`RaftCore`](`crate::core::RaftCore`) needs to shutdown.
    StorageError { error: StorageError<C> },

    /// Completion of an IO operation to local store.
    LocalIO { io_id: IOId<C> },

    /// Completion of purging log entries from storage.
    LogPurged { upto: LogIdOf<C> },

    /// Result of executing a command sent from network worker.
    ReplicationProgress {
        /// Identifies the replication stream that produced this progress.
        stream_id: StreamId,

        progress: replication::Progress<C>,

        /// The `InflightId` of the tracked operation that this progress answers, if any.
        ///
        /// - `Some(id)`: The progress answers an operation that the leader tracks as inflight: a
        ///   probe, a log stream, or a snapshot transfer. The leader acknowledges the operation, or
        ///   handles its conflict, only while it is still the inflight one with this `id`, so a
        ///   response to a superseded operation changes nothing. A request of a log stream carries
        ///   the stream's `id` even when it holds no entries, such as one that only advances the
        ///   commit index.
        ///
        /// - `None`: The progress answers an RPC that no inflight operation tracks: a heartbeat, or
        ///   an update of the commit index sent on its own. It acknowledges no inflight operation
        ///   and matches no conflict to one. But when log reversion is allowed, a conflict below
        ///   the known matching log still resets the matching log and ends a log stream.
        inflight_id: Option<InflightId>,
    },

    /// The replication task of `target` no longer reads the log for the plan the leader dropped.
    ///
    /// It answers [`Command::ClearReplicate`] with the same `inflight_id`, and it is sent only
    /// once no read for the dropped plan is running or can start.
    ///
    /// [`Command::ClearReplicate`]: crate::engine::Command::ClearReplicate
    ReplicateCleared {
        stream_id: StreamId,
        target: C::NodeId,
        inflight_id: InflightId,
    },

    HeartbeatProgress {
        stream_id: StreamId,
        sending_time: SendStamp<InstantOf<C>>,
        target: C::NodeId,
    },

    /// The replication task of the stream `stream_id` to `target`, which `RaftCore` closed, has
    /// stopped, so it reads the log no more.
    ///
    /// Until then the task may still be reading, and no log is purged, see
    /// [`RaftCore::lingering_replications`].
    ///
    /// [`RaftCore::lingering_replications`]: crate::core::RaftCore::lingering_replications
    ReplicationStopped { stream_id: StreamId, target: C::NodeId },

    /// Result of executing a command sent from a state machine worker.
    StateMachine { command_result: sm::CommandResult<C> },

    /// A leader's heartbeat timer reached the next heartbeat, waking RaftCore to send it.
    Tick {
        /// How many times the heartbeat timer has fired, counting from 1.
        i: u64,
    },

    /// The earliest pending linearizable read reached its deadline.
    PendingReadDeadlineReached,

    /// The instant at which this follower's election may be due is reached.
    ElectionDeadlineReached,
}

impl<C> Notification<C>
where C: RaftTypeConfig
{
    pub(crate) fn sm(command_result: sm::CommandResult<C>) -> Self {
        Self::StateMachine { command_result }
    }

    /// Returns the name of this notification variant.
    pub(crate) fn name(&self) -> NotificationName {
        match self {
            Self::VoteResponse { .. } => NotificationName::VoteResponse,
            Self::PreVoteResponse { .. } => NotificationName::PreVoteResponse,
            Self::HigherVote { .. } => NotificationName::HigherVote,
            Self::StorageError { .. } => NotificationName::StorageError,
            Self::LocalIO { .. } => NotificationName::LocalIO,
            Self::LogPurged { .. } => NotificationName::LogPurged,
            Self::ReplicationProgress { .. } => NotificationName::ReplicationProgress,
            Self::ReplicateCleared { .. } => NotificationName::ReplicateCleared,
            Self::ReplicationStopped { .. } => NotificationName::ReplicationStopped,
            Self::HeartbeatProgress { .. } => NotificationName::HeartbeatProgress,
            Self::StateMachine { .. } => NotificationName::StateMachine,
            Self::Tick { .. } => NotificationName::Tick,
            Self::PendingReadDeadlineReached => NotificationName::PendingReadDeadlineReached,
            Self::ElectionDeadlineReached => NotificationName::ElectionDeadlineReached,
        }
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
            Self::PreVoteResponse {
                target,
                resp,
                candidate_vote,
            } => {
                write!(
                    f,
                    "PreVoteResponse: from target={}, to pre_candidate_vote: {}, {}",
                    target, candidate_vote, resp
                )
            }
            Self::HigherVote {
                target,
                higher: new_vote,
                leader_vote: vote,
            } => {
                write!(
                    f,
                    "Seen a higher vote: target: {}, vote: {}, server_state_vote: {}",
                    target, new_vote, vote
                )
            }
            Self::StorageError { error } => write!(f, "StorageError: {}", error),
            Self::LocalIO { io_id } => write!(f, "IOFlushed: {}", io_id),
            Self::LogPurged { upto } => write!(f, "LogPurged: {}", upto),
            Self::ReplicationProgress {
                stream_id,
                progress,
                inflight_id,
            } => {
                write!(
                    f,
                    "{}, stream_id: {}, inflight_id: {}",
                    progress,
                    stream_id,
                    inflight_id.display()
                )
            }
            Self::HeartbeatProgress {
                stream_id,
                sending_time,
                target,
            } => {
                write!(
                    f,
                    "HeartbeatProgress: target={}, stream_id: {}, sending_time: {}",
                    target, stream_id, sending_time,
                )
            }
            Self::ReplicateCleared {
                stream_id,
                target,
                inflight_id,
            } => {
                write!(
                    f,
                    "ReplicateCleared: target={}, stream_id: {}, inflight_id: {}",
                    target, stream_id, inflight_id
                )
            }
            Self::ReplicationStopped { stream_id, target } => {
                write!(f, "ReplicationStopped: target={}, stream_id: {}", target, stream_id)
            }
            Self::StateMachine { command_result } => {
                write!(f, "{}", command_result)
            }
            Self::Tick { i } => {
                write!(f, "Tick {}", i)
            }
            Self::PendingReadDeadlineReached => {
                write!(f, "PendingReadDeadlineReached")
            }
            Self::ElectionDeadlineReached => {
                write!(f, "ElectionDeadlineReached")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Notification;
    use crate::engine::testing::UTConfig;
    use crate::progress::stream_id::StreamId;
    use crate::proposer::SendStamp;
    use crate::type_config::TypeConfigExt;

    /// The send time is displayed as a wall-clock time derived from the current time, so
    /// formatting the same stamp twice can give two different times: only its shape is checked.
    #[test]
    fn test_heartbeat_progress_display_labels_the_stream_id() {
        let notification = Notification::<UTConfig>::HeartbeatProgress {
            stream_id: StreamId::new(2),
            sending_time: SendStamp::new(UTConfig::<()>::now(), 3),
            target: 1,
        };

        let display = notification.to_string();
        let time = display
            .strip_prefix("HeartbeatProgress: target=1, stream_id: StreamId(2), sending_time: ")
            .and_then(|rest| rest.strip_suffix("(heartbeat_round=3)"))
            .unwrap_or_else(|| panic!("unexpected display: {display}"));
        let shape: String = time.chars().map(|c| if c.is_ascii_digit() { '9' } else { c }).collect();
        assert_eq!(shape, "99:99:99.999999", "send time in: {display}");
    }
}
