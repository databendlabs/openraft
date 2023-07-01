use std::fmt;

use crate::display_ext::DisplayOptionExt;
use crate::LogId;
use crate::MessageSummary;
use crate::NodeId;
use crate::Vote;

/// An RPC sent by candidates to gather votes (§5.2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct VoteRequest<NID: NodeId> {
    pub vote: Vote<NID>,
    pub last_log_id: Option<LogId<NID>>,
}

impl<NID: NodeId> fmt::Display for VoteRequest<NID> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{vote:{}, last_log:{}}}", self.vote, self.last_log_id.display(),)
    }
}

impl<NID: NodeId> MessageSummary<VoteRequest<NID>> for VoteRequest<NID> {
    fn summary(&self) -> String {
        self.to_string()
    }
}

impl<NID: NodeId> VoteRequest<NID> {
    pub fn new(vote: Vote<NID>, last_log_id: Option<LogId<NID>>) -> Self {
        Self { vote, last_log_id }
    }
}

/// The response to a `VoteRequest`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct VoteResponse<NID: NodeId> {
    /// vote after a node handling vote-request.
    /// Thus `resp.vote >= req.vote` always holds.
    pub vote: Vote<NID>,

    /// Will be true if the candidate received a vote from the responder.
    pub vote_granted: bool,

    /// The last log id stored on the remote voter.
    pub last_log_id: Option<LogId<NID>>,
}

impl<NID: NodeId> MessageSummary<VoteResponse<NID>> for VoteResponse<NID> {
    fn summary(&self) -> String {
        format!(
            "{{granted:{}, {}, last_log:{:?}}}",
            self.vote_granted,
            self.vote,
            self.last_log_id.map(|x| x.to_string())
        )
    }
}
