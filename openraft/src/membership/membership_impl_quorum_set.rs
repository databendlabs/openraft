use std::collections::BTreeSet;

use crate::Membership;
use crate::node::Node;
use crate::node::NodeId;
use crate::quorum::QuorumSet;

impl<NID, N> QuorumSet<NID> for Membership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    type Iter = std::collections::btree_set::IntoIter<NID>;

    fn is_quorum<'a, I>(&self, ids: I) -> bool
    where I: Iterator<Item = &'a NID> + Clone {
        for config in &self.configs {
            if !config.is_quorum(ids.clone()) {
                return false;
            }
        }
        true
    }

    fn ids(&self) -> Self::Iter {
        let mut ids = BTreeSet::new();
        for config in &self.configs {
            ids.extend(config.iter().cloned());
        }
        ids.into_iter()
    }
}

#[cfg(test)]
mod tests {
    use maplit::btreemap;
    use maplit::btreeset;

    use crate::Membership;
    use crate::quorum::QuorumSet;

    // `Membership` is the joint of its `configs`. `is_quorum`/`ids` read only `configs`,
    // so `nodes` is left empty in the quorum-set tests below.

    #[test]
    fn test_membership_is_quorum() -> anyhow::Result<()> {
        // Single config: simple majority.
        {
            let m12345 = Membership::<u64, ()> {
                configs: vec![btreeset! {1,2,3,4,5}],
                nodes: btreemap! {},
            };

            assert!(!m12345.is_quorum([0].iter()));
            assert!(!m12345.is_quorum([0, 1, 2].iter()));
            assert!(!m12345.is_quorum([6, 7, 8].iter()));
            assert!(m12345.is_quorum([1, 2, 3].iter()));
            assert!(m12345.is_quorum([3, 4, 5].iter()));
            assert!(m12345.is_quorum([1, 3, 4, 5].iter()));
        }

        // Joint config: must be a majority in every config.
        {
            let m12345_678 = Membership::<u64, ()> {
                configs: vec![btreeset! {1,2,3,4,5}, btreeset! {6,7,8}],
                nodes: btreemap! {},
            };

            assert!(!m12345_678.is_quorum([0].iter()));
            assert!(!m12345_678.is_quorum([0, 1, 2].iter()));
            assert!(!m12345_678.is_quorum([6, 7, 8].iter()));
            assert!(!m12345_678.is_quorum([1, 2, 3].iter()));
            assert!(m12345_678.is_quorum([1, 2, 3, 6, 7].iter()));
            assert!(m12345_678.is_quorum([1, 2, 3, 4, 7, 8].iter()));
        }

        Ok(())
    }

    #[test]
    fn test_membership_ids() -> anyhow::Result<()> {
        let m12345_678 = Membership::<u64, ()> {
            configs: vec![btreeset! {1,2,3,4,5}, btreeset! {4,5,6,7,8}],
            nodes: btreemap! {},
        };

        assert_eq!(btreeset! {1,2,3,4,5,6,7,8}, m12345_678.ids().collect());

        Ok(())
    }
}
