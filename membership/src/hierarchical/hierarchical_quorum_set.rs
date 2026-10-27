use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::btree_set;

use openraft::NodeId;
use openraft_macros::since;

use crate::errors::InvalidGroups;
use crate::quorum::QuorumIntersection;
use crate::quorum::QuorumSet;

/// One group of a [`HierarchicalQuorumSet`]: its members, which are voters and child groups, and
/// how many of them a set must reach to reach the group.
///
/// A set reaches a voter by holding it, and a child group by reaching enough members of that group
/// in turn. So the groups form a tree whose leaves are voters, and every level applies the same
/// test.
#[since(version = "0.10.0")]
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct Group<NID>
where NID: NodeId
{
    /// The number of members of the group that an accepted set must reach to reach the group.
    pub quorum_size: u64,

    /// The voters that the group holds directly.
    pub voters: BTreeSet<NID>,

    /// The child groups, by group ID.
    pub groups: BTreeMap<String, Group<NID>>,
}

impl<NID> Group<NID>
where NID: NodeId
{
    /// Returns a group of `voters`, which a set reaches by holding `quorum_size` of them.
    pub fn of_voters<I>(quorum_size: u64, voters: I) -> Self
    where I: IntoIterator<Item = NID> {
        Self {
            quorum_size,
            voters: voters.into_iter().collect(),
            groups: BTreeMap::new(),
        }
    }

    /// Returns a group of the child groups `groups`, each given with its ID, which a set reaches by
    /// reaching `quorum_size` of them. A later group replaces an earlier one with the same ID.
    pub fn of_groups<I, K>(quorum_size: u64, groups: I) -> Self
    where
        I: IntoIterator<Item = (K, Group<NID>)>,
        K: Into<String>,
    {
        let mut by_id = BTreeMap::new();
        for (id, group) in groups {
            by_id.insert(id.into(), group);
        }
        Self {
            quorum_size,
            voters: BTreeSet::new(),
            groups: by_id,
        }
    }

    /// Returns the number of members: the voters and the child groups.
    fn member_count(&self) -> u64 {
        let member_count = self.voters.len() + self.groups.len();
        member_count as u64
    }

    /// Returns whether `candidates` reach this group.
    fn is_reached_by(&self, candidates: &BTreeSet<&NID>) -> bool {
        let held_voters = self.voters.iter().filter(|id| candidates.contains(id));
        let held = held_voters.count() as u64;
        let reached_groups = self.groups.values().filter(|group| group.is_reached_by(candidates));
        let reached = reached_groups.count() as u64;
        held + reached >= self.quorum_size
    }

    /// Adds the voters of this group and of every group below it to `ids`.
    fn collect_voters(&self, ids: &mut BTreeSet<NID>) {
        ids.extend(self.voters.iter().cloned());
        for group in self.groups.values() {
            group.collect_voters(ids);
        }
    }

    /// Returns the group with the same members whose quorum size, here and in every group below,
    /// is `member_count - quorum_size + 1`.
    fn dual(&self) -> Self {
        let quorum_size = self.member_count() - self.quorum_size + 1;
        let mut groups = BTreeMap::new();
        for (id, group) in self.groups.iter() {
            groups.insert(id.clone(), group.dual());
        }
        Self {
            quorum_size,
            voters: self.voters.clone(),
            groups,
        }
    }

    /// Returns whether `self` and `other` hold the same voters and the same child group IDs, here
    /// and in every group below, with two quorum sizes that sum to more than the member count of
    /// each group.
    fn proves_intersection(&self, other: &Self) -> bool {
        let same_voters = self.voters == other.voters;
        let same_ids = self.groups.keys().eq(other.groups.keys());
        if !same_voters || !same_ids {
            return false;
        }

        let quorum_sizes = self.quorum_size + other.quorum_size;
        if quorum_sizes <= self.member_count() {
            return false;
        }

        let children = self.groups.values().zip(other.groups.values());
        for (group, other_group) in children {
            let proved = group.proves_intersection(other_group);
            if !proved {
                return false;
            }
        }
        true
    }
}

/// A quorum rule over a tree of groups: it accepts an ID set that reaches the root group.
///
/// A [`Group`] holds voters, child groups, or both: its members. A set reaches a group when the
/// voters of the group that it holds, plus the child groups that it reaches, number at least the
/// quorum size of the group.
///
/// Each group has an ID that is stable among its siblings, such as the name of a zone, and each
/// voter belongs to exactly one group. [`new()`](Self::new) checks that every group of the tree
/// holds a member and has `1 <= quorum_size <= member_count`, and that no voter appears twice in
/// the tree. So every group can be reached, and the set of all voters is a quorum.
///
/// The intersection check proves `Some(true)` for two rules over the same tree when their quorum
/// sizes sum to more than the member count of every group, as the read rule and the write rule of
/// a [`HierarchicalMembership`](crate::HierarchicalMembership) config do. It answers `None` for
/// every other pair.
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
/// // Zone a has racks r1 = {1, 2} and r2 = {3, 4}, zone b holds voters 5 and 6, and voter 7
/// // sits at the root. A quorum reaches 2 of zone a, zone b and voter 7. It reaches zone a
/// // through a voter in each rack, and zone b through both of its voters.
/// let a = Group::of_groups(2, [
///     ("r1", Group::of_voters(1, [1, 2])),
///     ("r2", Group::of_voters(1, [3, 4])),
/// ]);
/// let b = Group::of_voters(2, [5, 6]);
/// let root = Group {
///     quorum_size: 2,
///     voters: BTreeSet::from([7]),
///     groups: BTreeMap::from([("a".to_string(), a), ("b".to_string(), b)]),
/// };
/// let rule = HierarchicalQuorumSet::new(root).unwrap();
///
/// let accepts = rule.is_quorum([1, 3, 7].iter());
/// assert!(accepts);
/// // Rack r2 is lost, so only zone b is reached.
/// let accepts = rule.is_quorum([1, 2, 5, 6].iter());
/// assert!(!accepts);
///
/// // The quorums {1, 3, 7} and {2, 4, 5, 6} both reach zone a, through other voters of each
/// // rack. The check proves nothing, because the two quorum sizes of a rack sum to only its
/// // member count.
/// let intersects = rule.intersects_with(&rule);
/// assert_eq!(None, intersects);
/// ```
#[since(version = "0.10.0")]
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct HierarchicalQuorumSet<NID>
where NID: NodeId
{
    /// The root group, which holds every voter of the rule in its tree.
    root: Group<NID>,
}

impl<NID> HierarchicalQuorumSet<NID>
where NID: NodeId
{
    /// Builds a rule that accepts an ID set reaching `root`.
    ///
    /// Returns [`InvalidGroups`] for the first problem in a walk of the tree that visits a group
    /// before the groups below it, and sibling groups in ID order: a group that holds no member, a
    /// group without `1 <= quorum_size <= member_count`, or a voter that a visited group already
    /// holds.
    pub fn new(root: Group<NID>) -> Result<Self, InvalidGroups<NID>> {
        let mut group_of = BTreeMap::new();
        check_group(&root, &[], &mut group_of)?;

        let rule = Self { root };
        Ok(rule)
    }

    /// Returns the root group, which holds every voter of the rule in its tree.
    pub fn root(&self) -> &Group<NID> {
        &self.root
    }

    /// Returns the rule over the same tree that accepts exactly the sets that meet every quorum of
    /// `self`.
    ///
    /// A quorum of `self` that misses a set `S` takes its voters from outside `S`. Because no voter
    /// appears twice in the tree, it can reach the members of a group independently: each voter
    /// outside `S`, and each child group that `S` does not block. `S` blocks a group when it leaves
    /// fewer than `quorum_size` members reachable, so when it holds or blocks
    /// `member_count - quorum_size + 1` members: the voters that it holds and the child groups that
    /// it blocks. `S` meets every quorum exactly when it blocks the root group. So the result keeps
    /// the tree and gives every group that number as its quorum size.
    pub(crate) fn dual(&self) -> Self {
        let root = self.root.dual();
        Self { root }
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
        self.root.is_reached_by(&candidates)
    }

    fn ids(&self) -> Self::Iter {
        let mut ids = BTreeSet::new();
        self.root.collect_voters(&mut ids);
        ids.into_iter()
    }
}

/// Sufficient quorum intersection for two rules over the same tree.
///
/// A quorum of each rule reaches at least `quorum_size` members of the root group. When the two
/// quorum sizes sum to more than the member count, two quorums reach a common member. A common
/// voter is a shared voter, and a common child group repeats the argument one level down. So two
/// quorums share a voter when the two quorum sizes of every group sum to more than its member
/// count. A [`HierarchicalMembership`](crate::HierarchicalMembership) derives the read sizes of a
/// config so that every sum exceeds the member count by exactly 1. The check thus proves the read
/// rule and the write rule of every config. The check runs in O(n).
impl<NID> QuorumIntersection<HierarchicalQuorumSet<NID>> for HierarchicalQuorumSet<NID>
where NID: NodeId
{
    /// Return `Some(true)` when both rules hold the same voters and the same child group IDs in
    /// every group, and the two quorum sizes of every group sum to more than its member count.
    ///
    /// Return `None` otherwise, including for rules over different trees, such as after a voter
    /// moves to another group: the check proves nothing for them. This check never returns
    /// `Some(false)`.
    fn intersects_with(&self, other: &HierarchicalQuorumSet<NID>) -> Option<bool> {
        let proved = self.root.proves_intersection(&other.root);
        if proved { Some(true) } else { None }
    }
}

/// Returns an error unless `group`, at `path`, and every group below it hold a member and have
/// `1 <= quorum_size <= member_count`, and no voter appears twice. `group_of` maps each voter of
/// the groups visited before to the path of its group.
fn check_group<NID>(
    group: &Group<NID>,
    path: &[String],
    group_of: &mut BTreeMap<NID, Vec<String>>,
) -> Result<(), InvalidGroups<NID>>
where
    NID: NodeId,
{
    let member_count = group.member_count();
    if member_count == 0 {
        let empty = InvalidGroups::EmptyGroup { group: path.to_vec() };
        return Err(empty);
    }

    let size_fits = (1..=member_count).contains(&group.quorum_size);
    if !size_fits {
        let invalid = InvalidGroups::InvalidQuorumSize {
            group: path.to_vec(),
            quorum_size: group.quorum_size,
            member_count,
        };
        return Err(invalid);
    }

    for node_id in &group.voters {
        let Some(first) = group_of.insert(node_id.clone(), path.to_vec()) else {
            continue;
        };
        let shared = InvalidGroups::SharedVoter {
            node_id: node_id.clone(),
            first,
            second: path.to_vec(),
        };
        return Err(shared);
    }

    for (id, child) in group.groups.iter() {
        let mut child_path = path.to_vec();
        child_path.push(id.clone());
        check_group(child, &child_path, group_of)?;
    }
    Ok(())
}
