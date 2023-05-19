use crate::core::sm;
use crate::raft::VoteResponse;
use crate::replication;
use crate::MessageSummary;
use crate::RaftTypeConfig;
use crate::Vote;

/// A message coming from the internal components.
pub(crate) enum Notify<C>
where C: RaftTypeConfig
{
    VoteResponse {
        target: C::NodeId,
        resp: VoteResponse<C::NodeId>,

        /// The candidate that sent the vote request.
        ///
        /// A vote identifies a unique server state.
        sender_vote: Vote<C::NodeId>,
    },

    /// Seen a higher `vote`.
    HigherVote {
        /// The ID of the target node from which the new term was observed.
        target: C::NodeId,

        /// The higher vote observed.
        higher: Vote<C::NodeId>,

        /// The candidate or leader that sent the vote request.
        ///
        /// A vote identifies a unique server state.
        sender_vote: Vote<C::NodeId>,
        // TODO: need this?
        // /// The cluster this replication works for.
        // membership_log_id: Option<LogId<C::NodeId>>,
    },

    /// Result of executing a command sent from network worker.
    Network { response: replication::Response<C> },

    /// Result of executing a command sent from state machine worker.
    StateMachine { command_result: sm::CommandResult<C> },

    /// A tick event to wake up RaftCore to check timeout etc.
    Tick {
        /// ith tick
        i: u64,
    },
}

impl<C> Notify<C>
where C: RaftTypeConfig
{
    pub(crate) fn sm(command_result: sm::CommandResult<C>) -> Self {
        Self::StateMachine { command_result }
    }
}

impl<C> MessageSummary<Notify<C>> for Notify<C>
where C: RaftTypeConfig
{
    fn summary(&self) -> String {
        match self {
            Self::VoteResponse {
                target,
                resp,
                sender_vote: vote,
            } => {
                format!("VoteResponse: from: {}: {}, res-vote: {}", target, resp.summary(), vote)
            }
            Self::HigherVote {
                ref target,
                higher: ref new_vote,
                sender_vote: ref vote,
            } => {
                format!(
                    "Seen a higher vote: target: {}, vote: {}, server_state_vote: {}",
                    target, new_vote, vote
                )
            }
            Self::Network { response } => {
                format!("Replication command done: {}", response.summary())
            }
            Self::StateMachine { command_result } => {
                format!("StateMachine command done: {:?}", command_result)
            }
            Self::Tick { i } => {
                format!("Tick {}", i)
            }
        }
    }
}
