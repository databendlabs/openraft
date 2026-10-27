use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::errors::EmptyMembership;
use crate::errors::NodeMetadataChanged;
use crate::errors::NodeNotFound;
use crate::errors::Operation;
use crate::membership::NodeRole;
use crate::membership::RaftMembership;
use crate::membership::errors::UnprovenReadWriteIntersection;
use crate::membership::errors::WriteVoterWithoutReceiveLogs;
use crate::quorum::QuorumBridge;
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

    /// Returns [`WriteVoterWithoutReceiveLogs`] for the first write voter without
    /// [`NodeRole::ReceiveLogs`].
    ///
    /// OpenRaft replicates logs only to nodes with [`NodeRole::ReceiveLogs`], so such a voter never
    /// acknowledges a log. OpenRaft calls it before it uses the quorums of a membership that an
    /// application supplies.
    fn ensure_write_voters_receive_logs(&self) -> Result<(), WriteVoterWithoutReceiveLogs<Self::NodeId>> {
        let write_voter_ids = self.write_quorum_set().ids();
        for voter_id in write_voter_ids {
            if !self.receives_logs(&voter_id) {
                return Err(WriteVoterWithoutReceiveLogs { node_id: voter_id });
            }
        }

        Ok(())
    }

    /// Returns [`UnprovenReadWriteIntersection`] unless
    /// [`read_write_quorums_intersect()`](RaftMembership::read_write_quorums_intersect) returns
    /// `Some(true)`.
    ///
    /// OpenRaft calls it before it uses the quorums of a membership that an application supplies.
    fn ensure_read_write_quorums_intersect(&self) -> Result<(), UnprovenReadWriteIntersection> {
        let intersects = self.read_write_quorums_intersect();
        if intersects == Some(true) {
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
    /// Then [`RaftMembership::supports_direct_transition_to()`] decides.
    fn allows_direct_transition_to(&self, other: &Self) -> bool {
        if self.ensure_non_empty_config().is_err() {
            return false;
        }
        if other.ensure_non_empty_config().is_err() {
            return false;
        }

        self.supports_direct_transition_to(other)
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
mod raft_membership_ext_test;
