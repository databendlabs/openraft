use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::errors::EmptyMembership;
use crate::errors::NodeMetadataChanged;
use crate::errors::NodeNotFound;
use crate::errors::Operation;
use crate::errors::UnprovenReadWriteIntersection;
use crate::membership::NodeRole;
use crate::membership::RaftMembership;
use crate::quorum::QuorumBridge;
use crate::quorum::QuorumIntersection;
use crate::quorum::QuorumSet;

/// Operations derived from the required methods of [`RaftMembership`].
pub(crate) trait RaftMembershipExt: RaftMembership {
    /// Returns whether `node_id` is a known node of this configuration.
    fn contains(&self, node_id: &Self::NodeId) -> bool {
        let node = self.get_node(node_id);
        node.is_some()
    }

    /// Returns whether `node_id` may start an election under this configuration.
    fn is_campaigner(&self, node_id: &Self::NodeId) -> bool {
        let Some(node) = self.get_node(node_id) else {
            return false;
        };
        node.role.contains(NodeRole::Campaigner)
    }

    /// Returns whether `node_id` may receive replicated logs.
    fn receives_logs(&self, node_id: &Self::NodeId) -> bool {
        let Some(node) = self.get_node(node_id) else {
            return false;
        };
        node.role.contains(NodeRole::ReceiveLogs)
    }

    /// Returns whether `node_id` is in the read quorum set.
    fn is_read_voter(&self, node_id: &Self::NodeId) -> bool {
        let Some(node) = self.get_node(node_id) else {
            return false;
        };
        node.is_read_voter
    }

    /// Returns whether `node_id` is in the write quorum set.
    fn is_write_voter(&self, node_id: &Self::NodeId) -> bool {
        let Some(node) = self.get_node(node_id) else {
            return false;
        };
        node.is_write_voter
    }

    /// Returns whether `node_id` is a known node that receives logs and has no campaign or voter
    /// role.
    fn is_learner(&self, node_id: &Self::NodeId) -> bool {
        let Some(node) = self.get_node(node_id) else {
            return false;
        };
        node.is_learner()
    }

    /// Returns the IDs in the read or the write quorum set, in ascending order.
    fn voter_ids(&self) -> impl Iterator<Item = Self::NodeId> {
        let read_ids = self.read_quorum_set().ids();
        let write_ids = self.write_quorum_set().ids();
        let ids: BTreeSet<_> = read_ids.chain(write_ids).collect();
        ids.into_iter()
    }

    /// Returns the known node IDs outside the write quorum set.
    fn non_write_voter_ids(&self) -> impl Iterator<Item = Self::NodeId> {
        self.node_ids().filter(|id| !self.is_write_voter(id))
    }

    /// Returns [`NodeMetadataChanged`] for the first node ID that both configurations know but
    /// describe with a different node.
    ///
    /// A node ID that only one of the two configurations knows is skipped: adding a node and
    /// removing a node are not metadata changes.
    fn ensure_node_metadata_unchanged(&self, other: &Self) -> Result<(), NodeMetadataChanged<Self::NodeId>> {
        for node_id in self.node_ids() {
            let Some(node) = self.get_node(&node_id) else {
                continue;
            };
            let Some(other_node) = other.get_node(&node_id) else {
                continue;
            };

            if other_node.node != node.node {
                return Err(NodeMetadataChanged { node_id });
            }
        }

        Ok(())
    }

    /// Returns an error if the read or the write quorum set has no usable quorum.
    ///
    /// A quorum set is usable when the empty set is not a quorum and the set of all its IDs is a
    /// quorum. [`QuorumSet`] rules are upward-closed, so the second test passes exactly when some
    /// quorum exists. A joint quorum set with no component, or with an empty component, fails the
    /// second test, because it accepts no set.
    ///
    /// OpenRaft calls it before it uses the quorums of a membership that an application supplies.
    fn ensure_non_empty_config(&self) -> Result<(), EmptyMembership> {
        let quorum_sets = [self.read_quorum_set(), self.write_quorum_set()];
        for quorum_set in quorum_sets {
            let empty_is_quorum = quorum_set.is_quorum(std::iter::empty());
            if empty_is_quorum {
                return Err(EmptyMembership {});
            }

            let ids: Vec<_> = quorum_set.ids().collect();
            let all_is_quorum = quorum_set.is_quorum(ids.iter());
            if !all_is_quorum {
                return Err(EmptyMembership {});
            }
        }

        Ok(())
    }

    /// Returns [`NodeNotFound`] for the first quorum ID that has no node.
    ///
    /// OpenRaft calls it before it uses the quorums of a membership that an application supplies.
    fn ensure_voters_have_nodes(&self) -> Result<(), NodeNotFound<Self::NodeId>> {
        for voter_id in self.voter_ids() {
            if !self.contains(&voter_id) {
                return Err(NodeNotFound::new(voter_id, Operation::None));
            }
        }

        Ok(())
    }

    /// Returns [`UnprovenReadWriteIntersection`] unless [`QuorumIntersection::intersects_with()`]
    /// proves that every read quorum intersects every write quorum.
    ///
    /// With `allow_unproven`, it also accepts a configuration for which `intersects_with()` returns
    /// `None`. It rejects `Some(false)` either way.
    ///
    /// OpenRaft calls it before it uses the quorums of a membership that an application supplies.
    fn ensure_read_write_quorums_intersect(&self, allow_unproven: bool) -> Result<(), UnprovenReadWriteIntersection> {
        let read = self.read_quorum_set();
        let write = self.write_quorum_set();
        let intersects = read.intersects_with(write);

        let proven = intersects == Some(true);
        let trusted = allow_unproven && intersects.is_none();
        if proven || trusted {
            return Ok(());
        }

        Err(UnprovenReadWriteIntersection {
            membership: self.to_string(),
        })
    }

    /// Returns whether `other` can be appended directly after `self`, as one log entry without an
    /// intermediate joint configuration.
    ///
    /// Both configurations must pass [`ensure_non_empty_config()`](Self::ensure_non_empty_config).
    /// Then [`QuorumIntersection::intersects_with()`] must prove that every read quorum of `self`
    /// intersects every write quorum of `other`, and that every write quorum of `self` intersects
    /// every read quorum of `other`.
    fn supports_direct_transition_to(&self, other: &Self) -> bool {
        if self.ensure_non_empty_config().is_err() {
            return false;
        }
        if other.ensure_non_empty_config().is_err() {
            return false;
        }

        let read_meets_write = self.read_quorum_set().intersects_with(other.write_quorum_set());
        if read_meets_write != Some(true) {
            return false;
        }

        let write_meets_read = self.write_quorum_set().intersects_with(other.read_quorum_set());
        write_meets_read == Some(true)
    }

    /// Returns `goal` without the voters of `self` that are learners of `goal`.
    ///
    /// [`RaftMembershipEdit::apply()`](crate::membership::RaftMembershipEdit::apply) keeps a voter
    /// that an edit removes. OpenRaft calls this method when a membership change does not
    /// retain such voters as learners. A removed voter that keeps [`NodeRole::Campaigner`] in
    /// `goal` is not a learner, so it stays.
    fn drop_voters_demoted_to_learners(&self, goal: Self) -> Self {
        let mut nodes = Vec::new();
        for node_id in goal.node_ids() {
            // Safe unwrap(): `node_ids()` lists only the IDs that `get_node()` knows.
            let node = goal.get_node(&node_id).unwrap();
            let was_voter = self.is_read_voter(&node_id) || self.is_write_voter(&node_id);
            if was_voter && node.is_learner() {
                continue;
            }
            nodes.push((node_id, node.node.clone(), node.role));
        }

        let read = goal.read_quorum_set().clone();
        let write = goal.write_quorum_set().clone();
        Self::new(read, write, nodes)
    }

    /// Returns the joint configuration of `self` and `goal`.
    ///
    /// Its read quorum set is the [`joint_with()`](QuorumBridge::joint_with) of the read quorum
    /// sets of `self` and `goal`, and its write quorum set comes from the write quorum sets the
    /// same way. It knows every node of `goal` and every voter of `self`. A node takes its
    /// metadata from `goal` when `goal` knows it. A voter of `self` keeps its roles in `self` as
    /// well.
    fn joint_with(&self, goal: &Self) -> Self {
        let read = self.read_quorum_set().joint_with(goal.read_quorum_set().clone());
        let write = self.write_quorum_set().joint_with(goal.write_quorum_set().clone());

        let mut nodes = BTreeMap::new();
        for node_id in goal.node_ids() {
            // Safe unwrap(): `node_ids()` lists only the IDs that `get_node()` knows.
            let node = goal.get_node(&node_id).unwrap();
            nodes.insert(node_id, (node.node.clone(), node.role));
        }

        for voter_id in self.voter_ids() {
            // Safe unwrap(): a voter of `self` is a node that `get_node()` knows.
            let node = self.get_node(&voter_id).unwrap();
            let entry = nodes.entry(voter_id.clone());
            let (_, role) = entry.or_insert_with(|| (node.node.clone(), NodeRole::Empty));
            *role |= node.role;
        }

        let nodes = nodes.into_iter().map(|(node_id, (node, role))| (node_id, node, role));
        Self::new(read, write, nodes)
    }
}

impl<T> RaftMembershipExt for T where T: RaftMembership {}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::collections::BTreeSet;
    use std::fmt;

    use maplit::btreemap;
    use maplit::btreeset;

    use crate::errors::UnprovenReadWriteIntersection;
    use crate::membership::NodeRef;
    use crate::membership::NodeRole;
    use crate::membership::RaftMembership;
    use crate::membership::RaftMembershipExt;

    /// A configuration that stores the role of each node, so that a node outside the quorum sets
    /// can campaign. Its read and write quorum sets may differ.
    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    #[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
    struct RoleMembership {
        read: Vec<BTreeSet<u64>>,
        write: Vec<BTreeSet<u64>>,
        roles: BTreeMap<u64, NodeRole>,
    }

    impl fmt::Display for RoleMembership {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{:?}", self)
        }
    }

    impl RaftMembership for RoleMembership {
        type NodeId = u64;
        type Node = ();
        type QuorumSet = Vec<BTreeSet<u64>>;

        fn read_quorum_set(&self) -> &Self::QuorumSet {
            &self.read
        }

        fn write_quorum_set(&self) -> &Self::QuorumSet {
            &self.write
        }

        fn get_node(&self, node_id: &u64) -> Option<NodeRef<'_, ()>> {
            let role = *self.roles.get(node_id)?;
            let is_read_voter = self.read.iter().any(|ids| ids.contains(node_id));
            let is_write_voter = self.write.iter().any(|ids| ids.contains(node_id));
            let node_ref = NodeRef {
                node: &(),
                role,
                is_read_voter,
                is_write_voter,
            };
            Some(node_ref)
        }

        type Iter = std::vec::IntoIter<u64>;

        fn node_ids(&self) -> Self::Iter {
            let keys = self.roles.keys();
            let ids: Vec<u64> = keys.copied().collect();
            ids.into_iter()
        }

        fn new<I>(read_quorum_set: Self::QuorumSet, write_quorum_set: Self::QuorumSet, nodes: I) -> Self
        where I: IntoIterator<Item = (u64, (), NodeRole)> {
            let mut roles = BTreeMap::new();
            for (node_id, _node, role) in nodes {
                roles.insert(node_id, role);
            }

            RoleMembership {
                read: read_quorum_set,
                write: write_quorum_set,
                roles,
            }
        }
    }

    #[test]
    fn drop_voters_demoted_to_learners_keeps_campaigners() {
        let voter = NodeRole::Campaigner | NodeRole::ReceiveLogs;
        let learner = NodeRole::ReceiveLogs;
        let current = RoleMembership {
            read: vec![btreeset! {1, 2, 3}],
            write: vec![btreeset! {1, 2, 3}],
            roles: btreemap! {1 => voter, 2 => voter, 3 => voter, 4 => learner},
        };
        // Voters 2 and 3 leave the quorum sets: 2 keeps campaigning, and 3 becomes a learner.
        let goal = RoleMembership {
            read: vec![btreeset! {1}],
            write: vec![btreeset! {1}],
            roles: btreemap! {1 => voter, 2 => voter, 3 => learner, 4 => learner},
        };

        let res = current.drop_voters_demoted_to_learners(goal);

        let want = RoleMembership {
            read: vec![btreeset! {1}],
            write: vec![btreeset! {1}],
            roles: btreemap! {1 => voter, 2 => voter, 4 => learner},
        };
        assert_eq!(want, res);
    }

    #[test]
    fn ensure_read_write_quorums_intersect_accepts_unproven_only_when_allowed() {
        let with_quorum_sets = |read, write| RoleMembership {
            read,
            write,
            roles: BTreeMap::new(),
        };

        // Every majority of `{1,2}` intersects every majority of `{1,2,3}`.
        let proven = with_quorum_sets(vec![btreeset! {1, 2}], vec![btreeset! {1, 2, 3}]);

        let res = proven.ensure_read_write_quorums_intersect(false);
        assert_eq!(Ok(()), res);

        let res = proven.ensure_read_write_quorums_intersect(true);
        assert_eq!(Ok(()), res);

        // Read quorum `{1,2}` misses write quorum `{4,5}`.
        let disjoint = with_quorum_sets(vec![btreeset! {1, 2, 3}], vec![btreeset! {3, 4, 5}]);
        let want = Err(UnprovenReadWriteIntersection {
            membership: disjoint.to_string(),
        });

        let res = disjoint.ensure_read_write_quorums_intersect(false);
        assert_eq!(want, res);

        let res = disjoint.ensure_read_write_quorums_intersect(true);
        assert_eq!(want, res);

        // Every read quorum holds at least two of `{2,3,4,5}`, and every write quorum holds three
        // of them, so they intersect. No pair of configs shows it: majority `{1,2}` of `{1,2,3}`
        // and majority `{1,4}` of `{1,4,5}` each miss a majority of `{2,3,4,5}`, so
        // `intersects_with()` returns `None`.
        let read = vec![btreeset! {1, 2, 3}, btreeset! {1, 4, 5}];
        let write = vec![btreeset! {2, 3, 4, 5}];
        let unproven = with_quorum_sets(read, write);
        let want = Err(UnprovenReadWriteIntersection {
            membership: unproven.to_string(),
        });

        let res = unproven.ensure_read_write_quorums_intersect(false);
        assert_eq!(want, res);

        let res = unproven.ensure_read_write_quorums_intersect(true);
        assert_eq!(Ok(()), res);
    }

    #[test]
    fn joint_with_joins_read_and_write_quorum_sets_separately() {
        let voter = NodeRole::Campaigner | NodeRole::ReceiveLogs;
        let learner = NodeRole::ReceiveLogs;
        let current = RoleMembership {
            read: vec![btreeset! {1, 2}],
            write: vec![btreeset! {1, 2, 3}],
            roles: btreemap! {1 => voter, 2 => voter, 3 => voter, 4 => learner, 5 => learner},
        };
        let goal = RoleMembership {
            read: vec![btreeset! {4, 5}],
            write: vec![btreeset! {3, 4, 5}],
            roles: btreemap! {1 => learner, 2 => learner, 3 => voter, 4 => voter, 5 => voter},
        };

        let joint = current.joint_with(&goal);

        // The read quorum set of the joint holds the read configs of both sides, and its write
        // quorum set holds their write configs. Voters 1 and 2 keep their roles in `current`.
        let want = RoleMembership {
            read: vec![btreeset! {1, 2}, btreeset! {4, 5}],
            write: vec![btreeset! {1, 2, 3}, btreeset! {3, 4, 5}],
            roles: btreemap! {1 => voter, 2 => voter, 3 => voter, 4 => voter, 5 => voter},
        };
        assert_eq!(want, joint);
    }

    #[test]
    fn joint_with_joins_old_and_goal_quorum_sets() {
        let voter = NodeRole::Campaigner | NodeRole::ReceiveLogs;
        let learner = NodeRole::ReceiveLogs;
        let current = RoleMembership {
            read: vec![btreeset! {1, 2}],
            write: vec![btreeset! {1, 2, 3}],
            roles: btreemap! {1 => voter, 2 => voter, 3 => voter, 4 => learner},
        };
        let goal = RoleMembership {
            read: vec![btreeset! {1, 2, 4}],
            write: vec![btreeset! {1, 2, 4}],
            roles: btreemap! {1 => voter, 2 => voter, 3 => learner, 4 => voter},
        };

        let joint = current.joint_with(&goal);

        // Every read quorum of the joint is a quorum of `{1,2}`, so it holds 1 and 2 and meets every
        // write quorum of `current`. Voter 3 keeps its roles in `current`.
        let want = RoleMembership {
            read: vec![btreeset! {1, 2}, btreeset! {1, 2, 4}],
            write: vec![btreeset! {1, 2, 3}, btreeset! {1, 2, 4}],
            roles: btreemap! {1 => voter, 2 => voter, 3 => voter, 4 => voter},
        };
        assert_eq!(want, joint);
    }
}
