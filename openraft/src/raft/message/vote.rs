use std::borrow::Borrow;
use std::fmt;

use display_more::DisplayOptionExt;

use crate::RaftTypeConfig;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::VoteOf;

/// An RPC sent by candidates to gather votes (§5.2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct VoteRequest<C: RaftTypeConfig> {
    /// The candidate's vote requesting support.
    pub vote: VoteOf<C>,
    /// The candidate's last log id.
    pub last_log_id: Option<LogIdOf<C>>,
    /// If true this is a **pre-vote** probe (Raft §9.6): the candidate is testing
    /// whether it *could* win an election at `vote.term()` without bumping its
    /// persisted term. A recipient runs the same grant checks but does not
    /// persist/adopt the vote, so a node that cannot win never disturbs the
    /// cluster's term. Always `false` for a normal vote.
    #[cfg_attr(feature = "serde", serde(default))]
    pub pre_vote: bool,
}

impl<C> fmt::Display for VoteRequest<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{vote:{}, last_log:{}}}", self.vote, self.last_log_id.display(),)
    }
}

impl<C> VoteRequest<C>
where C: RaftTypeConfig
{
    /// Create a new vote request.
    pub fn new(vote: VoteOf<C>, last_log_id: Option<LogIdOf<C>>) -> Self {
        Self {
            vote,
            last_log_id,
            pre_vote: false,
        }
    }

    /// Create a new **pre-vote** probe request (does not bump the term).
    pub fn new_pre_vote(vote: VoteOf<C>, last_log_id: Option<LogIdOf<C>>) -> Self {
        Self {
            vote,
            last_log_id,
            pre_vote: true,
        }
    }
}

/// The response to a `VoteRequest`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct VoteResponse<C: RaftTypeConfig> {
    /// vote after a node handling vote-request.
    /// Thus, `resp.vote >= req.vote` always holds.
    ///
    /// `vote` that equals the candidate.vote does not mean the vote is granted.
    /// The `vote` may be updated when a previous Leader sees a higher vote.
    pub vote: VoteOf<C>,

    /// It is true if a node accepted and saved the VoteRequest.
    pub vote_granted: bool,

    /// The last log id stored on the remote voter.
    pub last_log_id: Option<LogIdOf<C>>,

    /// Echoes [`VoteRequest::pre_vote`]: true if this responds to a pre-vote
    /// probe. A pre-vote grant means "I would grant this vote" without the
    /// responder adopting the candidate's vote, so the tally keys on
    /// `vote_granted`, not on `vote` equality.
    #[cfg_attr(feature = "serde", serde(default))]
    pub pre_vote: bool,
}

impl<C> VoteResponse<C>
where C: RaftTypeConfig
{
    /// Create a new vote response.
    pub fn new(vote: impl Borrow<VoteOf<C>>, last_log_id: Option<LogIdOf<C>>, granted: bool) -> Self {
        Self {
            vote: vote.borrow().clone(),
            vote_granted: granted,
            last_log_id: last_log_id.map(|x| x.borrow().clone()),
            pre_vote: false,
        }
    }

    /// Create a new **pre-vote** response (echoes the pre-vote marker).
    pub fn new_pre_vote(vote: impl Borrow<VoteOf<C>>, last_log_id: Option<LogIdOf<C>>, granted: bool) -> Self {
        Self {
            vote: vote.borrow().clone(),
            vote_granted: granted,
            last_log_id: last_log_id.map(|x| x.borrow().clone()),
            pre_vote: true,
        }
    }

    /// Returns `true` if the response indicates that the target node has granted a vote to the
    /// candidate.
    pub fn is_granted_to(&self, candidate_vote: &VoteOf<C>) -> bool {
        &self.vote == candidate_vote
    }
}

impl<C> fmt::Display for VoteResponse<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{{{}, last_log:{:?}}}",
            self.vote,
            self.last_log_id.as_ref().map(|x| x.to_string())
        )
    }
}
