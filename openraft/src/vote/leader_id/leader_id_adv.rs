//! [`RaftLeaderId`] implementation that allows multiple leaders per term.

use std::fmt;

use crate::RaftTypeConfig;
use crate::vote::RaftLeaderId;

/// ID of a `leader`, allowing multiple leaders per term.
///
/// It includes the `term` and the `node_id`.
///
/// This is totally ordered to enable multiple leaders per term.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[derive(PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct LeaderId<C>
where C: RaftTypeConfig
{
    /// The term of the leader.
    pub term: C::Term,
    /// The node ID of the leader.
    pub node_id: C::NodeId,
}

impl<C> fmt::Display for LeaderId<C>
where C: RaftTypeConfig
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "T{}-N{}", self.term, self.node_id)
    }
}

/// The unique identifier of a leader that is already granted by a quorum in phase-1(voting).
///
/// [`CommittedLeaderId`] may contain less information than [`LeaderId`], because it implies the
/// constraint that **a quorum has granted it**.
///
/// For a total order `LeaderId`, the [`CommittedLeaderId`] is the same.
///
/// For a partial order `LeaderId`, we know that all the granted
/// leader-id must be a total order set. Therefore, once it is granted by a quorum, it only keeps
/// the information that makes leader-ids a correct total order set, e.g., in standard raft,
/// `voted_for: Option<node_id>` can be removed from `(term, voted_for)` once it is granted. This is
/// why standard raft stores just a `term` in log entry.
pub type CommittedLeaderId<C> = LeaderId<C>;

impl<C> RaftLeaderId<C> for LeaderId<C>
where C: RaftTypeConfig<LeaderId = Self>
{
    type Committed = Self;

    fn new(term: C::Term, node_id: C::NodeId) -> Self {
        Self { term, node_id }
    }

    fn term(&self) -> C::Term {
        self.term
    }

    fn node_id(&self) -> Option<&C::NodeId> {
        Some(&self.node_id)
    }

    fn to_committed(&self) -> Self::Committed {
        self.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::LeaderId;
    use crate::engine::testing::UTConfig;
    use crate::vote::RaftLeaderId;

    #[cfg(feature = "serde")]
    #[test]
    fn test_committed_leader_id_serde() -> anyhow::Result<()> {
        use crate::type_config::alias::CommittedLeaderIdOf;
        use crate::type_config::alias::LeaderIdOf;
        use crate::vote::RaftLeaderIdExt;

        let c = LeaderIdOf::<UTConfig>::new_committed(5, 10);
        let s = serde_json::to_string(&c)?;
        assert_eq!(r#"{"term":5,"node_id":10}"#, s);

        let c2: CommittedLeaderIdOf<UTConfig> = serde_json::from_str(&s)?;
        assert_eq!(LeaderIdOf::<UTConfig>::new_committed(5, 10), c2);

        Ok(())
    }

    #[test]
    fn test_adv_leader_id_partial_order() -> anyhow::Result<()> {
        #[allow(clippy::redundant_closure)]
        let lid = |term, node_id| LeaderId::<UTConfig>::new(term, node_id);

        // Compare term first
        assert!(lid(2, 2) > lid(1, 2));
        assert!(lid(1, 2) < lid(2, 2));

        // Equal term
        assert!(lid(2, 2) > lid(2, 1));
        assert!(lid(2, 1) < lid(2, 2));

        // Equal term, node_id
        assert!(lid(2, 2) == lid(2, 2));
        assert!(lid(2, 2) >= lid(2, 2));
        assert!(lid(2, 2) <= lid(2, 2));

        Ok(())
    }
}
