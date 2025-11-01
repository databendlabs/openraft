use std::cmp::Ordering;
use std::marker::PhantomData;

use crate::RaftTypeConfig;
use crate::vote::RaftLeaderId;

/// Provide comparison functions for [`RaftLeaderId`] implementations.
pub struct LeaderIdCompare<C>(PhantomData<C>);

impl<C> LeaderIdCompare<C>
where C: RaftTypeConfig
{
    /// Implements [`PartialOrd`] for LeaderId to enforce the standard Raft behavior of at most one
    /// leader per term.
    ///
    /// In standard Raft, each term can have at most one leader. This is enforced by making leader
    /// IDs with the same term incomparable (returning None), unless they refer to the same
    /// node.
    pub fn std<LID>(a: &LID, b: &LID) -> Option<Ordering>
    where LID: RaftLeaderId<C> {
        match a.term().cmp(&b.term()) {
            Ordering::Equal => match (a.node_id(), b.node_id()) {
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
            },
            cmp => Some(cmp),
        }
    }

    /// Implements [`PartialOrd`] for LeaderId to allow multiple leaders per term.
    pub fn adv<LID>(a: &LID, b: &LID) -> Option<Ordering>
    where LID: RaftLeaderId<C> {
        let res = (a.term(), a.node_id()).cmp(&(b.term(), b.node_id()));
        Some(res)
    }
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use crate::engine::testing::UTConfig;
    use crate::vote::RaftLeaderId;

    #[derive(Debug, PartialEq, Eq, Default, Clone, PartialOrd, derive_more::Display)]
    #[display("T{}-N{:?}", _0, _1)]
    #[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
    struct LeaderId(u64, Option<u64>);

    impl PartialEq<u64> for LeaderId {
        fn eq(&self, _other: &u64) -> bool {
            false
        }
    }

    impl PartialOrd<u64> for LeaderId {
        fn partial_cmp(&self, other: &u64) -> Option<Ordering> {
            self.0.partial_cmp(other)
        }
    }

    impl RaftLeaderId<UTConfig> for LeaderId {
        type Committed = u64;

        fn new(term: u64, node_id: u64) -> Self {
            Self(term, Some(node_id))
        }

        fn term(&self) -> u64 {
            self.0
        }

        fn node_id(&self) -> Option<&u64> {
            self.1.as_ref()
        }

        fn to_committed(&self) -> Self::Committed {
            self.0
        }
    }

    #[test]
    fn test_std_cmp() {
        use Ordering::*;

        use super::LeaderIdCompare as Cmp;

        let lid = |term, node_id| LeaderId(term, Some(node_id));
        let lid_none = |term| LeaderId(term, None);

        // Compare term first
        assert_eq!(Cmp::std(&lid(2, 2), &lid(1, 2)), Some(Greater));
        assert_eq!(Cmp::std(&lid(1, 2), &lid(2, 2)), Some(Less));

        // Equal term, Some > None
        assert_eq!(Cmp::std(&lid(2, 2), &lid_none(2)), Some(Greater));
        assert_eq!(Cmp::std(&lid_none(2), &lid(2, 2)), Some(Less));

        // Equal
        assert_eq!(Cmp::std(&lid(2, 2), &lid(2, 2)), Some(Equal));

        // Incomparable
        assert_eq!(Cmp::std(&lid(2, 2), &lid(2, 1)), None);
        assert_eq!(Cmp::std(&lid(2, 1), &lid(2, 2)), None);
        assert_eq!(Cmp::std(&lid(2, 2), &lid(2, 3)), None);
    }

    #[test]
    fn test_adv_cmp() {
        use Ordering::*;

        use super::LeaderIdCompare as Cmp;

        let lid = |term, node_id| LeaderId(term, Some(node_id));

        // Compare term first
        assert_eq!(Cmp::adv(&lid(2, 2), &lid(1, 2)), Some(Greater));
        assert_eq!(Cmp::adv(&lid(1, 2), &lid(2, 2)), Some(Less));

        // Equal term
        assert_eq!(Cmp::adv(&lid(2, 2), &lid(2, 1)), Some(Greater));
        assert_eq!(Cmp::adv(&lid(2, 1), &lid(2, 2)), Some(Less));

        // Equal term, node_id
        assert_eq!(Cmp::adv(&lid(2, 2), &lid(2, 2)), Some(Equal));
    }
}
