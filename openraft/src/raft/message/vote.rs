use std::borrow::Borrow;
use std::fmt;

use display_more::DisplayOptionExt;
use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::VoteOf;

/// An RPC sent by candidates to gather votes (§5.2).
#[since]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct VoteRequest<C: RaftTypeConfig> {
    /// The candidate's vote requesting support.
    pub vote: VoteOf<C>,
    /// The candidate's last log id.
    pub last_log_id: Option<LogIdOf<C>>,

    /// The vote of the Leader that authorized this election as a leadership transfer, or `None`
    /// for an ordinary election.
    ///
    /// A voter whose vote equals it processes the request even if the leader lease has not
    /// expired: the disruption the lease protects against is legitimate when the Leader itself
    /// initiates the election ("I have permission to disrupt the leader--it told me to!").
    /// See: Raft dissertation, section 4.2.3.
    ///
    /// A voter that follows another Leader still applies its lease. Otherwise a delayed transfer
    /// authorized by an older Leader could elect its target while a newer Leader holds a valid
    /// lease, and a lease read on that newer Leader could miss writes.
    #[since(
        version = "0.10.0",
        change = "replaced `leadership_transfer: bool` with the authorizing Leader's vote"
    )]
    #[since(version = "0.10.0")]
    #[cfg_attr(feature = "serde", serde(default))]
    pub leadership_transfer_from: Option<VoteOf<C>>,
}

impl<C> fmt::Display for VoteRequest<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{{vote:{}, last_log:{}, leadership_transfer_from:{}}}",
            self.vote,
            self.last_log_id.display(),
            self.leadership_transfer_from.display(),
        )
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
            leadership_transfer_from: None,
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

#[cfg(test)]
mod tests {

    #[cfg(feature = "serde")]
    #[test]
    fn test_vote_request_serde() {
        use crate::Vote;
        use crate::engine::testing::UTConfig;
        use crate::engine::testing::log_id;
        use crate::raft::VoteRequest;

        let req = VoteRequest::<UTConfig> {
            vote: Vote::new(2, 1),
            last_log_id: Some(log_id(1, 2, 3)),
            leadership_transfer_from: Some(Vote::new_committed(1, 2)),
        };

        let want = r#"{"vote":{"leader_id":{"term":2,"node_id":1},"committed":false},"last_log_id":{"leader_id":{"term":1,"node_id":2},"index":3},"leadership_transfer_from":{"leader_id":{"term":1,"node_id":2},"committed":true}}"#;

        let got = serde_json::to_string(&req).unwrap();
        assert_eq!(want, got);

        let decoded = serde_json::from_str::<VoteRequest<UTConfig>>(want).unwrap();
        assert_eq!(req, decoded);

        // A request from 0.10.0-alpha.22 onward carries the replaced `leadership_transfer: bool`.
        // It deserializes as an ordinary election, so a voter of this version applies its lease
        // instead of failing the RPC.
        let old = r#"{"vote":{"leader_id":{"term":2,"node_id":1},"committed":false},"last_log_id":{"leader_id":{"term":1,"node_id":2},"index":3},"leadership_transfer":true}"#;
        let ordinary = VoteRequest::<UTConfig> {
            leadership_transfer_from: None,
            ..req
        };

        let decoded = serde_json::from_str::<VoteRequest<UTConfig>>(old).unwrap();
        assert_eq!(ordinary, decoded);
    }
}
