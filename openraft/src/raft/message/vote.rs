use std::borrow::Borrow;
use std::fmt;

use display_more::DisplayOptionExt;
use openraft_macros::since;

use crate::RaftTypeConfig;
use crate::type_config::alias::LogIdOf;
use crate::type_config::alias::VoteOf;

/// A vote request, used by both RequestVote and Pre-Vote.
///
/// - RequestVote (§5.2): a candidate asks each voter to accept the vote that it has adopted for its
///   new term. A voter handles it with [`Raft::vote()`].
/// - Pre-Vote: without changing its own vote or term, a pre-candidate asks whether each voter would
///   accept a vote for the next term. A voter handles it with [`Raft::pre_vote()`].
///
/// A voter answers either one with a [`VoteResponse`].
///
/// [`Raft::vote()`]: crate::Raft::vote
/// [`Raft::pre_vote()`]: crate::Raft::pre_vote
#[since]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct VoteRequest<C: RaftTypeConfig> {
    /// The vote that the candidate or pre-candidate asks for.
    ///
    /// In RequestVote, it is the candidate's vote. In Pre-Vote, it is a vote for the
    /// pre-candidate's next term, which the pre-candidate has not adopted.
    pub vote: VoteOf<C>,

    /// The last log id of the candidate or pre-candidate.
    pub last_log_id: Option<LogIdOf<C>>,

    /// The vote of the Leader that authorized this election as a leadership transfer, or `None`
    /// for an ordinary election.
    ///
    /// A voter whose vote equals it processes a RequestVote even if the leader lease has not
    /// expired: the disruption the lease protects against is legitimate when the Leader itself
    /// initiates the election ("I have permission to disrupt the leader--it told me to!").
    /// See: Raft dissertation, section 4.2.3.
    ///
    /// A voter that follows another Leader still applies its lease. Otherwise a delayed transfer
    /// authorized by an older Leader could elect its target while a newer Leader holds a valid
    /// lease, and a lease read on that newer Leader could miss writes.
    ///
    /// Pre-Vote ignores this field: [`Raft::pre_vote()`] applies the leader lease to every
    /// request. The target of a leadership transfer campaigns without a Pre-Vote round, so a
    /// Pre-Vote request from Openraft holds `None`.
    ///
    /// [`Raft::pre_vote()`]: crate::Raft::pre_vote
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

/// The response to a [`VoteRequest`], used by both RequestVote and Pre-Vote.
///
/// A voter returns it from [`Raft::vote()`] or [`Raft::pre_vote()`], built from the voter's state
/// after it handled the request `req`. The fields below describe such a response.
///
/// The default implementations of [`RaftNetworkV2::pre_vote()`] and [`NetVote::pre_vote()`]
/// build a response without contacting the voter: a grant whose `vote` is `req.vote` and whose
/// `last_log_id` is `None`. No field of such a response comes from the voter.
///
/// [`Raft::vote()`]: crate::Raft::vote
/// [`Raft::pre_vote()`]: crate::Raft::pre_vote
/// [`RaftNetworkV2::pre_vote()`]: crate::network::RaftNetworkV2::pre_vote
/// [`NetVote::pre_vote()`]: crate::network::NetVote::pre_vote
#[since(version = "0.10.2", change = "the display shows `vote_granted`")]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct VoteResponse<C: RaftTypeConfig> {
    /// The voter's vote after it handled the request.
    ///
    /// A RequestVote grant carries `req.vote`. A denial, and every Pre-Vote response, carries the
    /// voter's current vote, which can be lower than, equal to, higher than, or not comparable
    /// with `req.vote`: a voter checks its leader lease and its last log id before it compares
    /// votes, and a Pre-Vote never changes the voter's vote. So a `vote` equal to `req.vote` does
    /// not mean the vote is granted, and a `vote` lower than `req.vote` is a valid response.
    ///
    /// A candidate or pre-candidate that receives a denial with a `vote` higher than its own adopts
    /// that vote, uncommitted, to catch up with the term of the cluster.
    pub vote: VoteOf<C>,

    /// Whether the voter grants `req.vote`.
    ///
    /// In RequestVote, `true` means that the voter accepted `req.vote` and saved it before it
    /// replied. In Pre-Vote, `true` means only that the voter would grant a RequestVote for
    /// `req.vote`: the voter changed and saved nothing.
    pub vote_granted: bool,

    /// The last log id stored on the voter.
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

    /// Returns `true` if the target node granted `candidate_vote` in this RequestVote response.
    ///
    /// A grant requires both [`vote_granted`](Self::vote_granted) and a [`vote`](Self::vote) equal
    /// to `candidate_vote`. A denial that carries the candidate's vote returns `false`, and so does
    /// a grant of another vote, such as a delayed reply to an earlier election.
    ///
    /// Use it only on a response from [`Raft::vote()`](crate::Raft::vote). A node grants a
    /// Pre-Vote without adopting the requested vote, so a Pre-Vote grant from
    /// [`Raft::pre_vote()`](crate::Raft::pre_vote) can carry another `vote`: check only its
    /// `vote_granted`.
    #[since(version = "0.10.2", change = "requires `vote_granted` to be `true`")]
    pub fn is_granted_to(&self, candidate_vote: &VoteOf<C>) -> bool {
        self.vote_granted && &self.vote == candidate_vote
    }
}

impl<C> fmt::Display for VoteResponse<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{{{}, last_log:{:?}, vote_granted:{}}}",
            self.vote,
            self.last_log_id.as_ref().map(|x| x.to_string()),
            self.vote_granted,
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

    #[test]
    fn test_vote_response_is_granted_to() {
        use crate::Vote;
        use crate::engine::testing::UTConfig;
        use crate::engine::testing::log_id;
        use crate::raft::VoteResponse;

        let candidate = Vote::new(2, 1);
        let last_log_id = Some(log_id(1, 2, 3));

        tracing::info!(%candidate, "--- a response with the candidate's vote grants it only with vote_granted");
        {
            let granted = VoteResponse::<UTConfig>::new(candidate, last_log_id, true);
            assert!(granted.is_granted_to(&candidate));

            let denied = VoteResponse::<UTConfig>::new(candidate, last_log_id, false);
            assert!(!denied.is_granted_to(&candidate));
        }

        tracing::info!(%candidate, "--- a response with another vote never grants the candidate's vote");
        {
            let same_leader_committed = Vote::new_committed(2, 1);
            let earlier_election = Vote::new(1, 1);

            for other in [same_leader_committed, earlier_election] {
                for vote_granted in [true, false] {
                    let resp = VoteResponse::<UTConfig>::new(other, last_log_id, vote_granted);
                    assert!(
                        !resp.is_granted_to(&candidate),
                        "vote: {other}, vote_granted: {vote_granted}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_vote_response_display() {
        use crate::Vote;
        use crate::engine::testing::UTConfig;
        use crate::engine::testing::log_id;
        use crate::raft::VoteResponse;

        let vote = Vote::new(2, 1);

        tracing::info!(%vote, "--- a grant and a denial with the same vote and last log id display their decisions");
        {
            let last_log_id = Some(log_id(1, 2, 3));

            let granted = VoteResponse::<UTConfig>::new(vote, last_log_id, true);
            assert_eq!(
                granted.to_string(),
                r#"{<T2-N1:->, last_log:Some("T1-N2.3"), vote_granted:true}"#
            );

            let denied = VoteResponse::<UTConfig>::new(vote, last_log_id, false);
            assert_eq!(
                denied.to_string(),
                r#"{<T2-N1:->, last_log:Some("T1-N2.3"), vote_granted:false}"#
            );
        }

        tracing::info!(%vote, "--- a response without a last log id displays it as None");
        {
            let denied = VoteResponse::<UTConfig>::new(vote, None, false);
            assert_eq!(denied.to_string(), "{<T2-N1:->, last_log:None, vote_granted:false}");
        }
    }
}
