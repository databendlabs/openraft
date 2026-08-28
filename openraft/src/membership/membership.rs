use core::fmt;
use std::collections::BTreeMap;
use std::collections::BTreeSet;

use openraft_macros::since;

use crate::ChangeMembers;
use crate::errors::EmptyMembership;
use crate::errors::MembershipError;
use crate::errors::NodeNotFound;
use crate::errors::Operation;
use crate::membership::IntoNodes;
use crate::node::Node;
use crate::node::NodeId;
use crate::quorum::Coherent;
use crate::quorum::FindCoherent;

/// The membership configuration of the cluster.
///
/// It could be a joint of one, two or more configs, i.e., a quorum is a node set that is superset
/// of a majority of every config.
#[since(
    version = "0.10.0",
    change = "replaced `C: RaftTypeConfig` with `NID: NodeId, N: Node`"
)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct Membership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Multi configs of members.
    ///
    /// AKA a joint config in original raft paper.
    pub(crate) configs: Vec<BTreeSet<NID>>,

    /// Additional info of all nodes, e.g., the connecting host and port.
    ///
    /// A node-id key that is in `nodes` but is not in `configs` is a **learner**.
    pub(crate) nodes: BTreeMap<NID, N>,
}

impl<NID, N> Default for Membership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn default() -> Self {
        Membership {
            configs: vec![],
            nodes: BTreeMap::new(),
        }
    }
}

impl<NID, N> From<BTreeMap<NID, N>> for Membership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn from(b: BTreeMap<NID, N>) -> Self {
        let member_ids = b.keys().cloned().collect::<BTreeSet<NID>>();
        Membership::new_unchecked(vec![member_ids], b)
    }
}

impl<NID, N> fmt::Display for Membership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{voters:[",)?;

        for (i, c) in self.configs.iter().enumerate() {
            if i > 0 {
                write!(f, ",",)?;
            }

            write!(f, "{{",)?;
            for (i, node_id) in c.iter().enumerate() {
                if i > 0 {
                    write!(f, ",",)?;
                }
                self.fmt_node(f, node_id)?;
            }
            write!(f, "}}")?;
        }
        write!(f, "]")?;

        let all_node_ids = self.nodes.keys().cloned().collect::<BTreeSet<_>>();
        let members = self.voter_ids().collect::<BTreeSet<_>>();

        write!(f, ", learners:[")?;

        for (learner_cnt, learner_id) in all_node_ids.difference(&members).enumerate() {
            if learner_cnt > 0 {
                write!(f, ",")?;
            }

            self.fmt_node(f, learner_id)?;
        }
        write!(f, "]}}")?;
        Ok(())
    }
}

// Public APIs
impl<NID, N> Membership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Create a new Membership from a joint config of voter-ids and a collection of all
    /// `Node` (voter nodes and learner nodes).
    ///
    /// A node id that is in `nodes` but is not in `config` is a **learner**.
    ///
    /// A node presents in `config` but not in `nodes` result in an error return.
    ///
    /// The `nodes` implements [`IntoNodes`] thus it can be `BTreeMap<NodeId, Node>` or
    /// `HashMap<NodeId,Node>` including all Voter and Learner nodes.
    pub fn new<T>(config: Vec<BTreeSet<NID>>, nodes: T) -> Result<Self, MembershipError<NID>>
    where T: IntoNodes<NID, N> {
        let m = Membership {
            configs: config,
            nodes: nodes.into_nodes(),
        };

        m.ensure_valid()?;
        Ok(m)
    }

    /// Create a new Membership with default nodes from voter configurations and a collection of all
    /// the node ids.
    ///
    /// A new [`Membership`] instance is built with `Node::default()`.
    ///
    /// # Arguments
    ///
    /// - `config`: Joint configuration containing sets of voter node IDs
    /// - `nodes`: Iterator of all node IDs in the cluster
    pub fn new_with_defaults<T>(config: Vec<BTreeSet<NID>>, nodes: T) -> Self
    where
        T: IntoIterator<Item = NID>,
        N: Default,
    {
        let voter_nodes = config.iter().flatten().cloned().map(|x| (x, N::default())).collect::<BTreeMap<_, _>>();

        let nodes = Self::extend_nodes(nodes.into_iter().map(|x| (x, N::default())).collect(), &voter_nodes);

        Membership { configs: config, nodes }
    }

    /// Returns reference to the joint config.
    ///
    /// Membership is defined by a joint of multiple configs.
    /// Each config is a vec of node-id.
    ///
    /// The returned `Vec` contains one or more configs. If there is only one config, it is in a
    /// uniform config, otherwise, it is in a joint consensus.
    pub fn get_joint_config(&self) -> &Vec<BTreeSet<NID>> {
        &self.configs
    }

    /// Returns true if this membership is in joint consensus, i.e. it has more than one config.
    pub(crate) fn is_joint(&self) -> bool {
        self.configs.len() > 1
    }

    /// Returns an Iterator of all nodes(voters and learners).
    pub fn nodes(&self) -> impl Iterator<Item = (&NID, &N)> {
        self.nodes.iter()
    }

    /// Get the node (either voter or learner) by node id.
    pub fn get_node(&self, node_id: &NID) -> Option<&N> {
        self.nodes.get(node_id)
    }

    /// Returns an Iterator of all voter node ids. Learners are not included.
    pub fn voter_ids(&self) -> impl Iterator<Item = NID> {
        self.configs.iter().flatten().cloned().collect::<BTreeSet<_>>().into_iter()
    }

    /// Returns an Iterator of all learner node ids. Voters are not included.
    pub fn learner_ids(&self) -> impl Iterator<Item = NID> + '_ {
        self.nodes.keys().filter(|x| !self.is_voter(x)).cloned()
    }
}

impl<NID, N> Membership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Format one node as `<node_id>:<node>`, or `<node_id>:None` if this membership has no such
    /// node.
    fn fmt_node(&self, f: &mut fmt::Formatter<'_>, node_id: &NID) -> fmt::Result {
        write!(f, "{node_id}:")?;
        match self.get_node(node_id) {
            Some(n) => write!(f, "{n:?}"),
            None => write!(f, "None"),
        }
    }

    /// Return true if the given node id is either a voter or a learner.
    pub(crate) fn contains(&self, node_id: &NID) -> bool {
        self.nodes.contains_key(node_id)
    }

    /// Check if the given `NodeId` exists and is a voter.
    pub(crate) fn is_voter(&self, node_id: &NID) -> bool {
        for c in self.configs.iter() {
            if c.contains(node_id) {
                return true;
            }
        }
        false
    }

    /// Create a new Membership the same as [`Self::new()`], but does not add the default
    /// value `Node::default()` if a voter id is not in `nodes`. Thus, it may create an invalid
    /// instance.
    pub(crate) fn new_unchecked<T>(configs: Vec<BTreeSet<NID>>, nodes: T) -> Self
    where T: IntoNodes<NID, N> {
        let nodes = nodes.into_nodes();
        Membership { configs, nodes }
    }

    /// Extends nodes btreemap with another.
    ///
    /// Node that present in `old` will **NOT** be replaced because changing the address of a node
    /// potentially breaks consensus guarantees.
    pub(crate) fn extend_nodes(old: BTreeMap<NID, N>, new: &BTreeMap<NID, N>) -> BTreeMap<NID, N> {
        let mut res = old;

        for (k, v) in new.iter() {
            if res.contains_key(k) {
                continue;
            }
            res.insert(k.clone(), v.clone());
        }

        res
    }

    /// Ensure the membership config is valid:
    /// - No empty sub-config in it.
    /// - Every voter has a corresponding Node.
    pub(crate) fn ensure_valid(&self) -> Result<(), MembershipError<NID>> {
        self.ensure_non_empty_config()?;
        self.ensure_voter_nodes().map_err(|nid| NodeNotFound::new(nid, Operation::None))?;
        Ok(())
    }

    /// Ensures that no sub-config in this joint config is empty.
    pub(crate) fn ensure_non_empty_config(&self) -> Result<(), EmptyMembership> {
        for c in self.get_joint_config().iter() {
            if c.is_empty() {
                return Err(EmptyMembership {});
            }
        }

        Ok(())
    }

    /// Ensures that this membership defines a quorum: it holds at least one sub-config, and none
    /// of them is empty.
    ///
    /// Zero sub-configs is as unusable as an empty sub-config: [`QuorumSet::is_quorum()`] iterates
    /// the sub-configs, so with none of them it returns true for every node set.
    ///
    /// [`Membership::default()`] carries exactly that value as the empty sentinel, and storage
    /// implementations round-trip it through [`Membership::new()`], so this check is not part of
    /// [`Membership::ensure_valid()`]. Only a path that reads quorums out of a caller-supplied
    /// membership calls it.
    ///
    /// [`QuorumSet::is_quorum()`]: crate::quorum::QuorumSet::is_quorum
    pub(crate) fn ensure_quorum_defined(&self) -> Result<(), EmptyMembership> {
        if self.get_joint_config().is_empty() {
            return Err(EmptyMembership {});
        }

        self.ensure_non_empty_config()
    }

    /// Ensures that every vote has a corresponding Node.
    ///
    /// If a voter is found not having a Node, it returns the voter node id in an `Err()`
    pub(crate) fn ensure_voter_nodes(&self) -> Result<(), NID> {
        for voter_id in self.voter_ids() {
            if !self.nodes.contains_key(&voter_id) {
                return Err(voter_id);
            }
        }

        Ok(())
    }

    /// Returns the first node id that both memberships know but describe with a different [`Node`].
    ///
    /// A node id that only one of the two memberships knows is skipped: adding a node and removing
    /// a node are not metadata changes.
    pub(crate) fn find_changed_node_metadata(&self, other: &Self) -> Option<NID> {
        for (node_id, node) in self.nodes.iter() {
            let Some(other_node) = other.nodes.get(node_id) else {
                continue;
            };

            if other_node != node {
                return Some(node_id.clone());
            }
        }

        None
    }

    // ---
    // Quorum related internal API
    // ---
    /// Returns true if one membership can be appended directly on top of the other, as a single
    /// log entry without an intermediate joint membership.
    ///
    /// The predicate is symmetric, and it accepts a transition only when quorum intersection
    /// follows from one of two cases:
    ///
    /// - Both memberships are uniform, i.e. each has exactly one voter set, and the two voter sets
    ///   differ by at most one node id. The majority sizes then leave no room for two disjoint
    ///   quorums.
    /// - Otherwise, the two memberships share an exactly equal voter set, which is the same
    ///   argument [`Coherent::is_coherent_with()`] makes for joint consensus.
    ///
    /// The rule is conservative: it rejects some transitions whose quorums do intersect. See
    /// [`UnsupportedMembershipTransition`] for examples.
    ///
    /// [`UnsupportedMembershipTransition`]: crate::errors::UnsupportedMembershipTransition
    pub(crate) fn is_direct_append_compatible_with(&self, other: &Self) -> bool {
        if self.ensure_quorum_defined().is_err() {
            return false;
        }
        if other.ensure_quorum_defined().is_err() {
            return false;
        }

        let both_uniform = self.configs.len() == 1 && other.configs.len() == 1;

        if both_uniform {
            let differing = self.configs[0].symmetric_difference(&other.configs[0]).count();
            return differing <= 1;
        }

        self.configs.is_coherent_with(&other.configs)
    }

    /// Returns the next coherent membership to change to, while the expected final membership is
    /// `goal`.
    ///
    /// `retain` specifies whether to retain the removed voters as learners, i.e., nodes that
    /// continue to receive log replication from the leader.
    ///
    /// E.g.(`cicj` is a joint membership of `ci` and `cj`):
    /// - `c1.next_step(c1)` returns `c1`
    /// - `c1.next_step(c2)` returns `c1c2`
    /// - `c1c2.next_step(c2)` returns `c2`
    /// - `c1c2.next_step(c1)` returns `c1`
    /// - `c1c2.next_step(c3)` returns `c2c3`
    ///
    /// With this method the membership change algo is simplified to:
    /// ```ignore
    /// while curr != goal {
    ///     let next = curr.next_coherent(goal);
    ///     change_membership(next);
    ///     curr = next;
    /// }
    /// ```
    pub(crate) fn next_coherent(&self, goal: BTreeSet<NID>, retain: bool) -> Self {
        let config = self.configs.find_coherent(goal);

        let mut nodes = self.nodes.clone();

        if !retain {
            let old_voter_ids = self.configs.iter().flatten().cloned().collect::<BTreeSet<_>>();
            let new_voter_ids = config.iter().flatten().cloned().collect::<BTreeSet<_>>();

            for node_id in old_voter_ids.difference(&new_voter_ids) {
                nodes.remove(node_id);
            }
        };

        Membership::new_unchecked(config, nodes)
    }

    /// Apply a change-membership request and return a new instance.
    ///
    /// It ensures that the returned instance is valid.
    ///
    /// `retain` specifies whether to retain the removed voters as learners, i.e., nodes that
    /// continue to receive log replication from the leader.
    pub(crate) fn change(mut self, change: ChangeMembers<NID, N>, retain: bool) -> Result<Self, MembershipError<NID>> {
        tracing::debug!("{}: change: {:?}", func_name!(), change);

        let Membership { mut configs, nodes } = self.clone().compute_target_membership(change);

        // Safe unwrap(): membership changes are only evaluated on an initialized leader.
        let target_voter_ids = configs.pop().unwrap();

        self.nodes = nodes;
        let new_membership = self.next_coherent(target_voter_ids, retain);

        tracing::debug!("new membership: {}", new_membership);

        new_membership.ensure_valid()?;

        Ok(new_membership)
    }

    /// Compute the target membership configuration by applying a membership change.
    ///
    /// This method:
    /// - Uses only the last config entry from the current membership. If there are multiple
    ///   entries, it indicates an ongoing joint consensus change. The last entry represents the
    ///   target configuration toward which the cluster is transitioning.
    /// - Applies the specified membership change to create a new target configuration
    /// - Returns a new `Membership` with the target voter IDs and nodes
    ///
    /// Note: This is an intermediate step in membership changes. The result may need to be
    /// transformed into a coherent configuration before being applied.
    fn compute_target_membership(mut self, change: ChangeMembers<NID, N>) -> Membership<NID, N> {
        let last = self.get_joint_config().last().cloned().unwrap_or_default();

        // `None` means the change does not touch the voter set, only `nodes`.
        let new_voter_ids: Option<BTreeSet<NID>> = match change {
            ChangeMembers::AddVoterIds(add_voter_ids) => Some(last.union(&add_voter_ids).cloned().collect()),
            ChangeMembers::AddVoters(add_voters) => {
                // Add nodes without overriding existent
                self.nodes = Self::extend_nodes(self.nodes, &add_voters);

                let add_voter_ids = add_voters.keys().cloned().collect::<BTreeSet<_>>();
                Some(last.union(&add_voter_ids).cloned().collect())
            }
            ChangeMembers::RemoveVoters(remove_voter_ids) => {
                Some(last.difference(&remove_voter_ids).cloned().collect())
            }
            ChangeMembers::ReplaceAllVoters(all_voter_ids) => Some(all_voter_ids),
            ChangeMembers::AddNodes(add_nodes) => {
                // When adding nodes, do not override existing node
                for (node_id, node) in add_nodes.into_iter() {
                    self.nodes.entry(node_id).or_insert(node);
                }
                None
            }
            ChangeMembers::SetNodes(set_nodes) => {
                for (node_id, node) in set_nodes.into_iter() {
                    self.nodes.insert(node_id, node);
                }
                None
            }
            ChangeMembers::RemoveNodes(remove_node_ids) => {
                for node_id in remove_node_ids.iter() {
                    self.nodes.remove(node_id);
                }
                None
            }
            ChangeMembers::ReplaceAllNodes(all_nodes) => {
                self.nodes = all_nodes;
                None
            }
            ChangeMembers::Batch(batch) => {
                // Each nested change already updated `configs` as needed.
                for change in batch {
                    self = self.compute_target_membership(change);
                }
                None
            }
        };

        if let Some(voter_ids) = new_voter_ids {
            self.configs = vec![voter_ids];
        }

        self
    }
}

#[cfg(test)]
mod tests {
    use maplit::btreemap;
    use maplit::btreeset;

    use crate::ChangeMembers;
    use crate::Membership;
    use crate::errors::ChangeMembershipError;
    use crate::errors::EmptyMembership;
    use crate::errors::LearnerNotFound;
    use crate::errors::MembershipError;
    use crate::type_config::alias::ChangeMembershipErrorOf;

    #[test]
    fn test_membership_ensure_voter_nodes() -> anyhow::Result<()> {
        let m = Membership::<u64, ()> {
            configs: vec![btreeset! {1,2}],
            nodes: btreemap! {1=>()},
        };
        assert_eq!(Err(2), m.ensure_voter_nodes());
        Ok(())
    }

    #[test]
    fn test_membership_change() -> anyhow::Result<()> {
        let m = || Membership::<u64, ()> {
            configs: vec![btreeset! {1,2}],
            nodes: btreemap! {1=>(),2=>(),3=>()},
        };

        // Add: no such learner
        {
            let res = m().change(ChangeMembers::AddVoterIds(btreeset! {4}), true);
            let err: ChangeMembershipErrorOf<crate::engine::testing::UTConfig> = res.unwrap_err().into();
            assert_eq!(
                ChangeMembershipError::LearnerNotFound(LearnerNotFound { node_id: 4 }),
                err
            );
        }

        // Add: ok
        {
            let res = m().change(ChangeMembers::AddVoterIds(btreeset! {3}), true);
            assert_eq!(
                Ok(Membership::<u64, ()> {
                    configs: vec![btreeset! {1,2}, btreeset! {1,2,3}],
                    nodes: btreemap! {1=>(),2=>(),3=>()}
                }),
                res
            );
        }

        // AddVoters
        {
            let res = m().change(ChangeMembers::AddVoters(btreemap! {5=>()}), true);
            assert_eq!(
                Ok(Membership::<u64, ()> {
                    configs: vec![btreeset! {1,2}, btreeset! {1,2,5}],
                    nodes: btreemap! {1=>(),2=>(),3=>(),5=>()}
                }),
                res
            );
        }

        // Remove: no such voter
        {
            let res = m().change(ChangeMembers::RemoveVoters(btreeset! {5}), true);
            assert_eq!(
                Ok(Membership::<u64, ()> {
                    configs: vec![btreeset! {1,2}],
                    nodes: btreemap! {1=>(),2=>(),3=>()}
                }),
                res
            );
        }

        // Remove: become empty
        {
            let res = m().change(ChangeMembers::RemoveVoters(btreeset! {1,2}), true);
            assert_eq!(Err(MembershipError::EmptyMembership(EmptyMembership {})), res);
        }

        // Remove: OK retain
        {
            let res = m().change(ChangeMembers::RemoveVoters(btreeset! {1}), true);
            assert_eq!(
                Ok(Membership::<u64, ()> {
                    configs: vec![btreeset! {1,2}, btreeset! {2}],
                    nodes: btreemap! {1=>(),2=>(),3=>()}
                }),
                res
            );
        }

        // Remove: OK, not retain; learner not removed
        {
            let res = m().change(ChangeMembers::RemoveVoters(btreeset! {1}), false);
            assert_eq!(
                Ok(Membership::<u64, ()> {
                    configs: vec![btreeset! {1,2}, btreeset! {2}],
                    nodes: btreemap! {1=>(),2=>(),3=>()}
                }),
                res
            );
        }

        // Remove: OK, not retain; learner removed
        {
            let mem = Membership::<u64, ()> {
                configs: vec![btreeset! {1,2}, btreeset! {2}],
                nodes: btreemap! {1=>(),2=>(),3=>()},
            };
            let res = mem.change(ChangeMembers::RemoveVoters(btreeset! {1}), false);
            assert_eq!(
                Ok(Membership::<u64, ()> {
                    configs: vec![btreeset! {2}],
                    nodes: btreemap! {2=>(),3=>()}
                }),
                res
            );
        }

        // Replace:
        {
            let res = m().change(ChangeMembers::ReplaceAllVoters(btreeset! {2}), false);
            assert_eq!(
                Ok(Membership::<u64, ()> {
                    configs: vec![btreeset! {1,2}, btreeset! {2}],
                    nodes: btreemap! {1=>(),2=>(),3=>()}
                }),
                res
            );
        }

        // AddNodes: existent voter
        {
            let res = m().change(ChangeMembers::AddNodes(btreemap! {2=>()}), false);
            assert_eq!(
                Ok(Membership::<u64, ()> {
                    configs: vec![btreeset! {1,2}],
                    nodes: btreemap! {1=>(),2=>(),3=>()}
                }),
                res
            );
        }

        // AddNodes: existent learner
        {
            let res = m().change(ChangeMembers::AddNodes(btreemap! {3=>()}), false);
            assert_eq!(
                Ok(Membership::<u64, ()> {
                    configs: vec![btreeset! {1,2}],
                    nodes: btreemap! {1=>(),2=>(),3=>()}
                }),
                res
            );
        }

        // AddNodes: Ok
        {
            let res = m().change(ChangeMembers::AddNodes(btreemap! {4=>()}), false);
            assert_eq!(
                Ok(Membership::<u64, ()> {
                    configs: vec![btreeset! {1,2}],
                    nodes: btreemap! {1=>(),2=>(),3=>(), 4=>()}
                }),
                res
            );
        }

        // SetNodes: Ok
        {
            let m = || Membership::<u64, u64> {
                configs: vec![btreeset! {1,2}],
                nodes: btreemap! {1=>1,2=>2,3=>3},
            };

            let res = m().change(ChangeMembers::SetNodes(btreemap! {3=>30, 4=>40}), false);
            assert_eq!(
                Ok(Membership::<u64, u64> {
                    configs: vec![btreeset! {1,2}],
                    nodes: btreemap! {1=>1,2=>2,3=>30, 4=>40}
                }),
                res
            );
        }

        // RemoveNodes: cannot remove node for voter
        {
            let res = m().change(ChangeMembers::RemoveNodes(btreeset! {2}), false);
            let err: ChangeMembershipErrorOf<crate::engine::testing::UTConfig> = res.unwrap_err().into();
            assert_eq!(
                ChangeMembershipError::LearnerNotFound(LearnerNotFound { node_id: 2 }),
                err
            );
        }

        // RemoveNodes: Ok
        {
            let res = m().change(ChangeMembers::RemoveNodes(btreeset! {3}), false);
            assert_eq!(
                Ok(Membership::<u64, ()> {
                    configs: vec![btreeset! {1,2}],
                    nodes: btreemap! {1=>(),2=>()}
                }),
                res
            );
        }

        // ReplaceAllNodes: Ok
        {
            let res = m().change(ChangeMembers::ReplaceAllNodes(btreemap! {1=>(),2=>(),4=>()}), false);
            assert_eq!(
                Ok(Membership::<u64, ()> {
                    configs: vec![btreeset! {1,2}],
                    nodes: btreemap! {1=>(),2=>(),4=>()}
                }),
                res
            );
        }

        Ok(())
    }

    /// Test membership change described by a batch operation.
    ///
    /// The batch operations add one voter and remove another.
    /// It still finishes in a two-step joint config change.
    #[test]
    fn test_membership_change_batch() -> anyhow::Result<()> {
        let m = || Membership::<u64, ()> {
            configs: vec![btreeset! {1,2}],
            nodes: btreemap! {1=>(),2=>(),3=>()},
        };

        let rm_2_add_5 = || {
            ChangeMembers::Batch(vec![
                ChangeMembers::RemoveVoters(btreeset! {2}),
                ChangeMembers::AddVoters(btreemap! {5=>()}),
            ])
        };

        let step1 = m().change(rm_2_add_5(), false)?;

        assert_eq!(step1, Membership::<u64, ()> {
            configs: vec![btreeset! {1,2}, btreeset! {1,5}],
            nodes: btreemap! {1=>(),2=>(),3=>(),5=>()}
        });

        let step2 = step1.change(rm_2_add_5(), false)?;

        assert_eq!(step2, Membership::<u64, ()> {
            configs: vec![btreeset! {1,5}],
            nodes: btreemap! {1=>(),3=>(), 5=>()}
        });

        Ok(())
    }
}
