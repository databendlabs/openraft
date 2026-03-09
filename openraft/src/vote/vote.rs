use std::cmp::Ordering;
use std::fmt::Formatter;

use openraft_macros::since;

use crate::vote::RaftLeaderId;
use crate::vote::RaftVote;
use crate::vote::ref_vote::RefVote;

/// `Vote` represent the privilege of a node.
#[since(
    version = "0.10.0",
    change = "from `Vote<C: RaftTypeConfig>` to `Vote<LID: RaftLeaderId>`"
)]
#[since(version = "0.8.0")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct Vote<LID>
where LID: RaftLeaderId
{
    /// The id of the node that tries to become the leader.
    pub leader_id: LID,

    /// Whether this vote has been committed (granted by a quorum).
    pub committed: bool,
}

impl<LID> PartialOrd for Vote<LID>
where LID: RaftLeaderId
{
    #[inline]
    fn partial_cmp(&self, other: &Vote<LID>) -> Option<Ordering> {
        let self_ref = RefVote::new(&self.leader_id, self.committed);
        let other_ref = RefVote::new(&other.leader_id, other.committed);
        PartialOrd::partial_cmp(&self_ref, &other_ref)
    }
}

impl<LID> std::fmt::Display for Vote<LID>
where LID: RaftLeaderId
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let ref_vote = RefVote::new(&self.leader_id, self.committed);
        ref_vote.fmt(f)
    }
}

impl<LID> RaftVote for Vote<LID>
where LID: RaftLeaderId
{
    type LeaderId = LID;

    fn from_leader_id(leader_id: LID, committed: bool) -> Self {
        Self { leader_id, committed }
    }

    fn leader_id(&self) -> &LID {
        &self.leader_id
    }

    fn is_committed(&self) -> bool {
        self.committed
    }
}

impl<LID> Vote<LID>
where LID: RaftLeaderId
{
    /// Create a new uncommitted vote for the given term and node.
    pub fn new(term: LID::Term, node_id: LID::NodeId) -> Self {
        Self {
            leader_id: LID::new(term, node_id),
            committed: false,
        }
    }

    /// Create a new committed vote for the given term and node.
    pub fn new_committed(term: LID::Term, node_id: LID::NodeId) -> Self {
        Self {
            leader_id: LID::new(term, node_id),
            committed: true,
        }
    }

    /// Mark this vote as committed (deprecated).
    #[deprecated(note = "use `into_committed()` instead", since = "0.10.0")]
    pub fn commit(&mut self) {
        self.committed = true
    }

    /// Check if this vote has been committed.
    pub fn is_committed(&self) -> bool {
        self.committed
    }

    /// Return the `LeaderId` this vote represents for.
    ///
    /// The leader may or may not be granted by a quorum.
    pub fn leader_id(&self) -> &LID {
        &self.leader_id
    }
}

#[cfg(test)]
#[allow(clippy::nonminimal_bool)]
mod tests {
    mod feature_no_single_term_leader {
        use crate::Vote;
        use crate::engine::testing::UTConfig;
        use crate::engine::testing::UTLeaderId;

        #[cfg(feature = "serde")]
        #[test]
        fn test_vote_serde() -> anyhow::Result<()> {
            let v = Vote::new(1, 2);
            let s = serde_json::to_string(&v)?;
            assert_eq!(r#"{"leader_id":{"term":1,"node_id":2},"committed":false}"#, s);

            let v2: Vote<UTLeaderId> = serde_json::from_str(&s)?;
            assert_eq!(v, v2);

            Ok(())
        }

        #[test]
        fn test_vote_total_order() -> anyhow::Result<()> {
            #[allow(clippy::redundant_closure)]
            let vote = |term, node_id| Vote::<UTLeaderId>::new(term, node_id);

            #[allow(clippy::redundant_closure)]
            let committed = |term, node_id| Vote::<UTLeaderId>::new_committed(term, node_id);

            // Compare term first
            assert!(vote(2, 2) > vote(1, 2));
            assert!(vote(1, 2) < vote(2, 2));

            // Equal term
            assert!(vote(2, 2) > vote(2, 1));
            assert!(vote(2, 1) < vote(2, 2));

            // Equal term, node_id
            assert!(vote(2, 2) == vote(2, 2));
            assert!(vote(2, 2) >= vote(2, 2));
            assert!(vote(2, 2) <= vote(2, 2));

            assert!(committed(2, 2) > vote(2, 2));
            assert!(vote(2, 2) < committed(2, 2));
            Ok(())
        }

        #[test]
        fn test_to_committed_leader_id() -> anyhow::Result<()> {
            use crate::type_config::alias::LeaderIdOf;
            use crate::vote::RaftLeaderId;
            use crate::vote::raft_vote::RaftVoteExt;

            let vote = Vote::<UTLeaderId>::new(1, 2);
            assert_eq!(None, vote.try_to_committed_leader_id());

            let committed = Vote::<UTLeaderId>::new_committed(1, 2);
            let leader_id = committed.try_to_committed_leader_id();
            let expected = LeaderIdOf::<UTConfig>::new(1, 2).to_committed();
            assert_eq!(Some(expected), leader_id);

            Ok(())
        }
    }

    mod feature_single_term_leader {
        use crate::Vote;
        use crate::vote::leader_id_std::LeaderId;

        type TCLeaderId = LeaderId<u64, u64>;

        #[cfg(feature = "serde")]
        #[test]
        fn test_vote_serde() -> anyhow::Result<()> {
            let v = Vote::<TCLeaderId>::new(1, 2);
            let s = serde_json::to_string(&v)?;
            assert_eq!(r#"{"leader_id":{"term":1,"voted_for":2},"committed":false}"#, s);

            let v2: Vote<TCLeaderId> = serde_json::from_str(&s)?;
            assert_eq!(v, v2);

            Ok(())
        }

        #[test]
        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        fn test_vote_partial_order() -> anyhow::Result<()> {
            #[allow(clippy::redundant_closure)]
            let vote = |term, node_id| Vote::<TCLeaderId>::new(term, node_id);

            #[allow(clippy::redundant_closure)]
            let committed = |term, node_id| Vote::<TCLeaderId>::new_committed(term, node_id);

            // Compare term first
            assert!(vote(2, 2) > vote(1, 2));
            assert!(vote(2, 2) >= vote(1, 2));
            assert!(vote(1, 2) < vote(2, 2));
            assert!(vote(1, 2) <= vote(2, 2));

            // Committed greater than non-committed if leader_id is incomparable
            assert!(committed(2, 2) > vote(2, 2));
            assert!(committed(2, 2) >= vote(2, 2));
            assert!(committed(2, 1) > vote(2, 2));
            assert!(committed(2, 1) >= vote(2, 2));

            // Lower term committed is not greater
            assert!(!(committed(1, 1) > vote(2, 1)));
            assert!(!(committed(1, 1) >= vote(2, 1)));

            // Compare to itself
            assert!(committed(1, 1) >= committed(1, 1));
            assert!(committed(1, 1) <= committed(1, 1));
            assert!(committed(1, 1) == committed(1, 1));

            // Equal
            assert!(vote(2, 2) == vote(2, 2));
            assert!(vote(2, 2) >= vote(2, 2));
            assert!(vote(2, 2) <= vote(2, 2));

            // Incomparable
            assert!(!(vote(2, 2) > vote(2, 3)));
            assert!(!(vote(2, 2) >= vote(2, 3)));
            assert!(!(vote(2, 2) < vote(2, 3)));
            assert!(!(vote(2, 2) <= vote(2, 3)));
            assert!(!(vote(2, 2) == vote(2, 3)));

            // Incomparable committed: returns None, not panic
            assert!(!(committed(2, 2) > committed(2, 3)));
            assert!(!(committed(2, 2) >= committed(2, 3)));
            assert!(!(committed(2, 2) < committed(2, 3)));
            assert!(!(committed(2, 2) <= committed(2, 3)));
            assert!(!(committed(2, 2) == committed(2, 3)));
            assert_eq!(committed(2, 2).partial_cmp(&committed(2, 3)), None);

            Ok(())
        }

        #[test]
        fn test_to_committed_leader_id() -> anyhow::Result<()> {
            use crate::vote::RaftLeaderId;
            use crate::vote::raft_vote::RaftVoteExt;

            let vote = Vote::<TCLeaderId>::new(1, 2);
            assert_eq!(None, vote.try_to_committed_leader_id());

            let committed = Vote::<TCLeaderId>::new_committed(1, 2);
            let leader_id = committed.try_to_committed_leader_id();
            let expected = LeaderId {
                term: 1,
                voted_for: Some(2),
            }
            .to_committed();
            assert_eq!(Some(expected), leader_id);

            Ok(())
        }
    }
}
