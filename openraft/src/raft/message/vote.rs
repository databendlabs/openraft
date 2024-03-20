use std::fmt;

use crate::display_ext::DisplayOptionExt;
use crate::LogId;
use crate::MessageSummary;
use crate::RaftTypeConfig;
use crate::Vote;

/// An RPC sent by candidates to gather votes (§5.2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct VoteRequest<C: RaftTypeConfig> {
    pub vote: Vote<C::NodeId>,
    pub last_log_id: Option<LogId<C::NodeId>>,
}

impl<C> fmt::Display for VoteRequest<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{vote:{}, last_log:{}}}", self.vote, self.last_log_id.display(),)
    }
}

impl<C> MessageSummary<VoteRequest<C>> for VoteRequest<C>
where C: RaftTypeConfig
{
    fn summary(&self) -> String {
        self.to_string()
    }
}

impl<C> VoteRequest<C>
where C: RaftTypeConfig
{
    pub fn new(vote: Vote<C::NodeId>, last_log_id: Option<LogId<C::NodeId>>) -> Self {
        Self { vote, last_log_id }
    }
}

/// The response to a `VoteRequest`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct VoteResponse<C: RaftTypeConfig> {
    /// vote after a node handling vote-request.
    /// Thus `resp.vote >= req.vote` always holds.
    pub vote: Vote<C::NodeId>,

    /// Will be true if the candidate received a vote from the responder.
    pub vote_granted: bool,

    /// The last log id stored on the remote voter.
    pub last_log_id: Option<LogId<C::NodeId>>,
}

impl<C> MessageSummary<VoteResponse<C>> for VoteResponse<C>
where C: RaftTypeConfig
{
    fn summary(&self) -> String {
        format!(
            "{{granted:{}, {}, last_log:{:?}}}",
            self.vote_granted,
            self.vote,
            self.last_log_id.map(|x| x.to_string())
        )
    }
}
