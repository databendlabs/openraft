use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::btree_set;

use openraft::NodeId;
use openraft_macros::since;

use crate::errors::InvalidGroups;
use crate::quorum::QuorumIntersection;
use crate::quorum::QuorumSet;

/// One group of a [`HierarchicalQuorumSet`]: its voters, and how many of them a set must hold to
/// reach the group.
#[since(version = "0.10.0")]
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct Group<NID>
where NID: NodeId
{
    /// The voters of the group.
    pub voters: BTreeSet<NID>,

    /// The number of voters of the group that an accepted set must hold to reach the group.
    pub quorum_size: u64,
}

/// A quorum rule over disjoint groups of voters: it accepts an ID set that reaches at least
/// `quorum_groups` groups. A set reaches a group when it holds at least as many voters of the
/// group as the quorum size of the group.
///
/// Each group has a stable ID, such as the name of a zone, and each voter belongs to exactly one
/// group. [`new()`](Self::new) checks that the layout holds a group, that every group holds a
/// voter, and that no two groups share a voter. It also checks that
/// `1 <= quorum_groups <= group_count`, and that every group has a quorum size with
/// `1 <= quorum_size <= voter_count`. So every group can be reached, and the set of all voters is
/// a quorum.
///
/// The intersection check proves `Some(true)` for two rules over the same groups when their
/// thresholds sum to more than the counts at both levels, as the read rule and the write rule of a
/// [`HierarchicalMembership`](crate::HierarchicalMembership) config do. It answers `None` for every
/// other pair.
///
/// # Examples
///
/// ```
/// use std::collections::BTreeMap;
/// use std::collections::BTreeSet;
///
/// use openraft::membership::{QuorumIntersection, QuorumSet};
/// use openraft_membership::quorum::{Group, HierarchicalQuorumSet};
///
/// // Three zones of three voters. A quorum holds 2 voters in each of 2 zones.
/// let groups = BTreeMap::from([
///     ("a".to_string(), Group { voters: BTreeSet::from([1, 2, 3]), quorum_size: 2 }),
///     ("b".to_string(), Group { voters: BTreeSet::from([4, 5, 6]), quorum_size: 2 }),
///     ("c".to_string(), Group { voters: BTreeSet::from([7, 8, 9]), quorum_size: 2 }),
/// ]);
/// let rule = HierarchicalQuorumSet::new(2, groups).unwrap();
///
/// // Zone c is lost, and so is voter 1 of zone a.
/// let accepts = rule.is_quorum([2, 3, 4, 5].iter());
/// assert!(accepts);
/// // Only zone a holds 2 voters of this set.
/// let accepts = rule.is_quorum([1, 2, 4, 7].iter());
/// assert!(!accepts);
///
/// // Two quorums share a zone, and a voter in it.
/// let intersects = rule.intersects_with(&rule);
/// assert_eq!(Some(true), intersects);
/// ```
#[since(version = "0.10.0")]
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct HierarchicalQuorumSet<NID>
where NID: NodeId
{
    /// The voters and the quorum size of each group, by group ID.
    groups: BTreeMap<String, Group<NID>>,

    /// The number of groups that an accepted set must reach.
    quorum_groups: u64,
}

impl<NID> HierarchicalQuorumSet<NID>
where NID: NodeId
{
    /// Builds a rule that accepts an ID set reaching at least `quorum_groups` of `groups`. A set
    /// reaches a group when it holds at least `quorum_size` voters of the group.
    ///
    /// `groups` maps each group ID to the voters and the quorum size of the group.
    ///
    /// Returns [`InvalidGroups`] when `groups` holds no group, when a group holds no voter, or when
    /// two groups share a voter. Also returns it unless `1 <= quorum_groups <= group_count`, and
    /// unless every group has `1 <= quorum_size <= voter_count`.
    pub fn new(quorum_groups: u64, groups: BTreeMap<String, Group<NID>>) -> Result<Self, InvalidGroups<NID>> {
        check_layout(&groups)?;

        let group_count = groups.len() as u64;
        let groups_fit = (1..=group_count).contains(&quorum_groups);
        if !groups_fit {
            let invalid = InvalidGroups::InvalidQuorumGroups {
                quorum_groups,
                group_count,
            };
            return Err(invalid);
        }
        check_quorum_sizes(&groups)?;

        let rule = Self { groups, quorum_groups };
        Ok(rule)
    }

    /// Returns the voters and the quorum size of each group, by group ID.
    pub fn groups(&self) -> &BTreeMap<String, Group<NID>> {
        &self.groups
    }

    /// Returns the number of groups that an accepted set must reach.
    pub fn quorum_groups(&self) -> u64 {
        self.quorum_groups
    }

    /// Returns the rule over the same layout that accepts exactly the sets that meet every quorum
    /// of `self`.
    ///
    /// A quorum of `self` that misses a set `S` takes its voters from outside `S`. Because the
    /// groups are disjoint, it can reach each group that keeps `quorum_size` voters outside `S`.
    /// `S` blocks a group when it holds `voter_count - quorum_size + 1` of its voters. `S` meets
    /// every quorum exactly when it blocks at least `group_count - quorum_groups + 1` groups, so
    /// that fewer than `quorum_groups` groups stay reachable. These two numbers are the thresholds
    /// of the result.
    pub(crate) fn dual(&self) -> Self {
        let group_count = self.groups.len() as u64;
        let quorum_groups = group_count - self.quorum_groups + 1;

        let mut groups = BTreeMap::new();
        for (group, Group { voters, quorum_size }) in self.groups.iter() {
            let voter_count = voters.len() as u64;
            let quorum_size = voter_count - quorum_size + 1;
            let voters = voters.clone();
            groups.insert(group.clone(), Group { voters, quorum_size });
        }

        Self { groups, quorum_groups }
    }
}

impl<NID> QuorumSet for HierarchicalQuorumSet<NID>
where NID: NodeId
{
    type Id = NID;
    type Iter = btree_set::IntoIter<NID>;

    fn is_quorum<'a, I>(&self, ids: I) -> bool
    where
        NID: 'a,
        I: Iterator<Item = &'a NID> + Clone,
    {
        let candidates: BTreeSet<&NID> = ids.collect();

        let mut reached = 0;
        for Group { voters, quorum_size } in self.groups.values() {
            let held_voters = voters.iter().filter(|id| candidates.contains(id));
            let held = held_voters.count() as u64;
            if held < *quorum_size {
                continue;
            }

            reached += 1;
            if reached >= self.quorum_groups {
                return true;
            }
        }
        false
    }

    fn ids(&self) -> Self::Iter {
        let mut ids = BTreeSet::new();
        for Group { voters, .. } in self.groups.values() {
            ids.extend(voters.iter().cloned());
        }
        ids.into_iter()
    }
}

/// Sufficient quorum intersection for two rules over the same groups.
///
/// A quorum of each rule reaches at least `quorum_groups` groups. When the two `quorum_groups` sum
/// to more than the group count, two quorums reach a common group. When the two quorum sizes of
/// every group sum to more than its voter count, two quorums share a voter in each group that both
/// reach. A [`HierarchicalMembership`](crate::HierarchicalMembership) derives the read thresholds
/// of a config so that both sums exceed their bounds by exactly 1. The check thus proves the read
/// rule and the write rule of every config. The check runs in O(n).
impl<NID> QuorumIntersection<HierarchicalQuorumSet<NID>> for HierarchicalQuorumSet<NID>
where NID: NodeId
{
    /// Return `Some(true)` when both rules hold the same groups, their `quorum_groups` sum to more
    /// than the group count, and the two quorum sizes of every group sum to more than its voter
    /// count.
    ///
    /// Return `None` otherwise, including for rules over different groups, such as after a voter
    /// moves to another group: the check proves nothing for them. This check never returns
    /// `Some(false)`.
    fn intersects_with(&self, other: &HierarchicalQuorumSet<NID>) -> Option<bool> {
        let layout = self.groups.iter().map(|(group, Group { voters, .. })| (group, voters));
        let other_layout = other.groups.iter().map(|(group, Group { voters, .. })| (group, voters));
        let same_layout = layout.eq(other_layout);
        if !same_layout {
            return None;
        }

        let group_count = self.groups.len() as u64;
        let quorum_groups = self.quorum_groups + other.quorum_groups;
        if quorum_groups <= group_count {
            return None;
        }

        for (group, Group { voters, quorum_size }) in self.groups.iter() {
            let voter_count = voters.len() as u64;
            // Safe index: both rules hold the same group IDs.
            let quorum_sizes = quorum_size + other.groups[group].quorum_size;
            if quorum_sizes <= voter_count {
                return None;
            }
        }
        Some(true)
    }
}

/// Returns an error unless `groups` holds a group, every group holds a voter, and no two groups
/// share a voter.
fn check_layout<NID>(groups: &BTreeMap<String, Group<NID>>) -> Result<(), InvalidGroups<NID>>
where NID: NodeId {
    if groups.is_empty() {
        return Err(InvalidGroups::NoGroup);
    }

    let mut group_of = BTreeMap::new();
    for (group, Group { voters, .. }) in groups {
        if voters.is_empty() {
            let empty = InvalidGroups::EmptyGroup { group: group.clone() };
            return Err(empty);
        }

        for node_id in voters {
            let Some(first) = group_of.insert(node_id, group) else {
                continue;
            };
            let shared = InvalidGroups::SharedVoter {
                node_id: node_id.clone(),
                first: first.clone(),
                second: group.clone(),
            };
            return Err(shared);
        }
    }
    Ok(())
}

/// Returns an error unless every group of `groups` has `1 <= quorum_size <= voter_count`.
fn check_quorum_sizes<NID>(groups: &BTreeMap<String, Group<NID>>) -> Result<(), InvalidGroups<NID>>
where NID: NodeId {
    for (group, Group { voters, quorum_size }) in groups {
        let voter_count = voters.len() as u64;
        let size_fits = (1..=voter_count).contains(quorum_size);
        if !size_fits {
            let invalid = InvalidGroups::InvalidQuorumSize {
                group: group.clone(),
                quorum_size: *quorum_size,
                voter_count,
            };
            return Err(invalid);
        }
    }
    Ok(())
}
