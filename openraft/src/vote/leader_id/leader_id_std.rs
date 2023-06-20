use std::cmp::Ordering;
use std::fmt;
use std::marker::PhantomData;

use crate::NodeId;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct LeaderId<NID>
where NID: NodeId
{
    pub term: u64,

    pub voted_for: Option<NID>,
}

impl<NID: NodeId> PartialOrd for LeaderId<NID> {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match PartialOrd::partial_cmp(&self.term, &other.term) {
            Some(Ordering::Equal) => {
                //
                match (&self.voted_for, &other.voted_for) {
                    (None, None) => Some(Ordering::Equal),
                    (Some(_), None) => Some(Ordering::Greater),
                    (None, Some(_)) => Some(Ordering::Less),
                    (Some(a), Some(b)) => {
                        if a == b {
                            Some(Ordering::Equal)
                        } else {
                            None
                        }
                    }
                }
            }
            cmp => cmp,
        }
    }
}

impl<NID: NodeId> LeaderId<NID> {
    pub fn new(term: u64, node_id: NID) -> Self {
        Self {
            term,
            voted_for: Some(node_id),
        }
    }

    pub fn get_term(&self) -> u64 {
        self.term
    }

    pub fn voted_for(&self) -> Option<NID> {
        self.voted_for
    }

    #[allow(clippy::wrong_self_convention)]
    pub(crate) fn to_committed(&self) -> CommittedLeaderId<NID> {
        CommittedLeaderId::new(self.term, NID::default())
    }

    /// Return if it is the same leader as the committed leader id.
    ///
    /// A committed leader may have less info than a non-committed.
    pub(crate) fn is_same_as_committed(&self, other: &CommittedLeaderId<NID>) -> bool {
        self.term == other.term
    }
}

impl<NID: NodeId> std::fmt::Display for LeaderId<NID> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{:?}", self.term, self.voted_for)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[derive(PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct CommittedLeaderId<NID> {
    pub term: u64,
    p: PhantomData<NID>,
}

impl<NID: NodeId> std::fmt::Display for CommittedLeaderId<NID> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.term)
    }
}

impl<NID: NodeId> CommittedLeaderId<NID> {
    pub fn new(term: u64, node_id: NID) -> Self {
        let _ = node_id;
        Self { term, p: PhantomData }
    }
}

#[cfg(test)]
mod tests {
    use crate::CommittedLeaderId;
    use crate::LeaderId;

    #[cfg(feature = "serde")]
    #[test]
    fn test_committed_leader_id_serde() -> anyhow::Result<()> {
        let c = CommittedLeaderId::<u32>::new(5, 10);
        let s = serde_json::to_string(&c)?;
        assert_eq!(r#"5"#, s);

        let c2: CommittedLeaderId<u32> = serde_json::from_str(&s)?;
        assert_eq!(CommittedLeaderId::new(5, 0), c2);

        Ok(())
    }

    #[test]
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    fn test_leader_id_partial_order() -> anyhow::Result<()> {
        #[allow(clippy::redundant_closure)]
        let lid = |term, node_id| LeaderId::<u64>::new(term, node_id);

        let lid_none = |term| LeaderId::<u64> { term, voted_for: None };

        // Compare term first
        assert!(lid(2, 2) > lid(1, 2));
        assert!(lid(1, 2) < lid(2, 2));

        // Equal term, Some > None
        assert!(lid(2, 2) > lid_none(2));
        assert!(lid_none(2) < lid(2, 2));

        // Equal
        assert!(lid(2, 2) == lid(2, 2));
        assert!(lid(2, 2) >= lid(2, 2));
        assert!(lid(2, 2) <= lid(2, 2));

        // Incomparable
        assert!(!(lid(2, 2) > lid(2, 3)));
        assert!(!(lid(2, 2) < lid(2, 3)));
        assert!(!(lid(2, 2) == lid(2, 3)));

        Ok(())
    }
}
