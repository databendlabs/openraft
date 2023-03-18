use std::fmt;

use crate::NodeId;

/// LeaderId is identifier of a `leader`.
///
/// In raft spec that in a term there is at most one leader, thus a `term` is enough to
/// differentiate leaders. That is why raft uses `(term, index)` to uniquely identify a log
/// entry.
///
/// Under this dirty simplification, a `Leader` is actually identified by `(term,
/// voted_for:Option<node_id>)`.
/// By introducing `LeaderId {term, node_id}`, things become easier to understand.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[derive(PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct LeaderId<NID>
where NID: NodeId
{
    pub term: u64,
    pub node_id: NID,
}

impl<NID: NodeId> LeaderId<NID> {
    pub fn new(term: u64, node_id: NID) -> Self {
        Self { term, node_id }
    }

    pub fn get_term(&self) -> u64 {
        self.term
    }

    pub fn voted_for(&self) -> Option<NID> {
        Some(self.node_id)
    }

    #[allow(clippy::wrong_self_convention)]
    pub(crate) fn to_committed(&self) -> CommittedLeaderId<NID> {
        *self
    }

    /// Return if it is the same leader as the committed leader id.
    ///
    /// A committed leader may have less info than a non-committed.
    pub(crate) fn is_same_as_committed(&self, other: &CommittedLeaderId<NID>) -> bool {
        self == other
    }
}

impl<NID: NodeId> std::fmt::Display for LeaderId<NID> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.term, self.node_id)
    }
}

/// The unique identifier of a leader that is already granted by a quorum in phase-1(voting).
///
/// [`CommittedLeaderId`] may contains less information than [`LeaderId`], because it implies the
/// constraint that **a quorum has granted it**.
///
/// For a total order `LeaderId`, the [`CommittedLeaderId`] is the same.
///
/// For a partial order `LeaderId`, we know that all of the granted
/// leader-id must be a total order set. Therefor once it is granted by a quorum, it only keeps the
/// information that makes leader-ids a correct total order set, e.g., in standard raft, `voted_for:
/// Option<node_id>` can be removed from `(term, voted_for)` once it is granted. This is why
/// standard raft stores just a `term` in log entry.
pub type CommittedLeaderId<NID> = LeaderId<NID>;

#[cfg(test)]
mod tests {
    use crate::LeaderId;

    #[cfg(feature = "serde")]
    #[test]
    fn test_committed_leader_id_serde() -> anyhow::Result<()> {
        use crate::CommittedLeaderId;

        let c = CommittedLeaderId::<u32>::new(5, 10);
        let s = serde_json::to_string(&c)?;
        assert_eq!(r#"{"term":5,"node_id":10}"#, s);

        let c2: CommittedLeaderId<u32> = serde_json::from_str(&s)?;
        assert_eq!(CommittedLeaderId::new(5, 10), c2);

        Ok(())
    }

    #[test]
    fn test_leader_id_partial_order() -> anyhow::Result<()> {
        #[allow(clippy::redundant_closure)]
        let lid = |term, node_id| LeaderId::<u64>::new(term, node_id);

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
