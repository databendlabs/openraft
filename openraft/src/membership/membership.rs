use core::fmt;
use std::collections::BTreeMap;
use std::collections::BTreeSet;

use maplit::btreemap;

use crate::error::ChangeMembershipError;
use crate::error::EmptyMembership;
use crate::error::LearnerNotFound;
use crate::membership::NodeRole;
use crate::node::Node;
use crate::quorum::AsJoint;
use crate::quorum::FindCoherent;
use crate::quorum::Joint;
use crate::quorum::QuorumSet;
use crate::ChangeMembers;
use crate::MessageSummary;
use crate::NodeId;

/// Convert types into a map of `Node`.
pub trait IntoNodes<NID, N>
where
    N: Node,
    NID: NodeId,
{
    fn has_nodes(&self) -> bool;
    fn node_ids(&self) -> Vec<NID>;
    fn into_nodes(self) -> BTreeMap<NID, N>;
}

impl<NID, N> IntoNodes<NID, N> for ()
where
    N: Node,
    NID: NodeId,
{
    fn has_nodes(&self) -> bool {
        false
    }

    fn node_ids(&self) -> Vec<NID> {
        vec![]
    }

    fn into_nodes(self) -> BTreeMap<NID, N> {
        btreemap! {}
    }
}

impl<NID, N> IntoNodes<NID, N> for BTreeSet<NID>
where
    N: Node,
    NID: NodeId,
{
    fn has_nodes(&self) -> bool {
        false
    }

    fn node_ids(&self) -> Vec<NID> {
        self.iter().copied().collect()
    }

    fn into_nodes(self) -> BTreeMap<NID, N> {
        self.into_iter().map(|node_id| (node_id, N::default())).collect()
    }
}

impl<NID, N> IntoNodes<NID, N> for Option<BTreeSet<NID>>
where
    N: Node,
    NID: NodeId,
{
    fn has_nodes(&self) -> bool {
        true
    }

    fn node_ids(&self) -> Vec<NID> {
        match self {
            None => {
                vec![]
            }
            Some(bs) => bs.iter().copied().collect(),
        }
    }

    fn into_nodes(self) -> BTreeMap<NID, N> {
        match self {
            None => BTreeMap::new(),
            Some(s) => s.into_iter().map(|node_id| (node_id, N::default())).collect(),
        }
    }
}

impl<NID, N> IntoNodes<NID, N> for BTreeMap<NID, N>
where
    N: Node,
    NID: NodeId,
{
    fn has_nodes(&self) -> bool {
        true
    }

    fn node_ids(&self) -> Vec<NID> {
        self.keys().copied().collect()
    }

    fn into_nodes(self) -> BTreeMap<NID, N> {
        self
    }
}

/// The membership configuration of the cluster.
///
/// It could be a joint of one, two or more configs, i.e., a quorum is a node set that is superset
/// of a majority of every config.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct Membership<NID, N>
where
    N: Node,
    NID: NodeId,
{
    /// Multi configs of members.
    ///
    /// AKA a joint config in original raft paper.
    configs: Vec<BTreeSet<NID>>,

    /// Additional info of all nodes, e.g., the connecting host and port.
    ///
    /// A node-id key that is in `nodes` but is not in `configs` is a **learner**.
    nodes: BTreeMap<NID, N>,
}

impl<NID, N> From<BTreeMap<NID, N>> for Membership<NID, N>
where
    N: Node,
    NID: NodeId,
{
    fn from(b: BTreeMap<NID, N>) -> Self {
        let member_ids = b.keys().cloned().collect::<BTreeSet<NID>>();
        Membership::new_with_nodes(vec![member_ids], b)
    }
}

impl<NID, N> fmt::Display for Membership<NID, N>
where
    N: Node,
    NID: NodeId,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.summary())
    }
}

impl<NID, N> MessageSummary<Membership<NID, N>> for Membership<NID, N>
where
    N: Node,
    NID: NodeId,
{
    fn summary(&self) -> String {
        let mut res = vec!["members:[".to_string()];
        for (i, c) in self.configs.iter().enumerate() {
            if i > 0 {
                res.push(",".to_string());
            }

            res.push("{".to_string());
            for (i, node_id) in c.iter().enumerate() {
                if i > 0 {
                    res.push(",".to_string());
                }
                res.push(format!("{}", node_id));

                let n = self.get_node(node_id).map(|x| format!("{:?}", x)).unwrap_or_else(|| "None".to_string());
                res.push(format!(":{{{}}}", n));
            }
            res.push("}".to_string());
        }
        res.push("]".to_string());

        let all_node_ids = self.nodes.keys().cloned().collect::<BTreeSet<_>>();
        let members = self.voter_ids().collect::<BTreeSet<_>>();

        res.push(",learners:[".to_string());
        for (learner_cnt, learner_id) in all_node_ids.difference(&members).enumerate() {
            if learner_cnt > 0 {
                res.push(",".to_string());
            }

            res.push(format!("{}", learner_id));

            let n = self.get_node(learner_id).map(|x| format!("{:?}", x)).unwrap_or_else(|| "None".to_string());
            res.push(format!(":{{{}}}", n));
        }
        res.push("]".to_string());
        res.join("")
    }
}

impl<NID, N> Membership<NID, N>
where
    N: Node,
    NID: NodeId,
{
    /// Create a new Membership of multiple configs(joint) and optionally a set of learner node ids.
    ///
    /// A node id that is in `nodes` but is not in `configs` is a **learner**.
    ///
    /// An node id present in `configs` but not in `nodes` is filled with default value.
    pub fn new<T>(configs: Vec<BTreeSet<NID>>, nodes: T) -> Self
    where T: IntoNodes<NID, N> {
        let voter_ids = configs.as_joint().ids().collect::<BTreeSet<_>>();
        let nodes = Self::extend_nodes(nodes.into_nodes(), &voter_ids.into_nodes());

        Membership { configs, nodes }
    }

    /// Create a new Membership of multiple configs and optional node infos.
    ///
    /// The node infos `nodes` can be:
    /// - a simple `()`, if there are no non-voter(learner) nodes,
    /// - `BTreeSet<NodeId>` provides learner node ids whose `Node` data are `Node::default()`,
    /// - `BTreeMap<NodeId, Node>` provides nodes for every node id. Node ids that are not in
    ///   `configs` are learners.
    pub(crate) fn new_with_nodes<T>(configs: Vec<BTreeSet<NID>>, nodes: T) -> Self
    where T: IntoNodes<NID, N> {
        let nodes = nodes.into_nodes();
        Membership { configs, nodes }
    }

    /// Extends nodes btreemap with another.
    ///
    /// Node that present in `old` will **NOT** be replaced because changing the address of a node
    /// potentially breaks consensus guarantee.
    pub(crate) fn extend_nodes(old: BTreeMap<NID, N>, new: &BTreeMap<NID, N>) -> BTreeMap<NID, N> {
        let mut res = old;

        for (k, v) in new.iter() {
            if res.contains_key(k) {
                continue;
            }
            res.insert(*k, v.clone());
        }

        res
    }

    /// Check to see if the config is currently in joint consensus.
    pub fn is_in_joint_consensus(&self) -> bool {
        self.configs.len() > 1
    }

    /// Ensure the membership config is valid:
    /// - No empty sub-config in it.
    /// - Every voter has a corresponding Node.
    pub(crate) fn ensure_valid(&self) -> Result<(), ChangeMembershipError<NID>> {
        self.ensure_non_empty_config()?;
        self.ensure_voter_nodes().map_err(|nid| LearnerNotFound { node_id: nid })?;
        Ok(())
    }

    /// Ensures that none of the sub config in this joint config are empty.
    pub(crate) fn ensure_non_empty_config(&self) -> Result<(), EmptyMembership> {
        for c in self.get_joint_config().iter() {
            if c.is_empty() {
                return Err(EmptyMembership {});
            }
        }

        Ok(())
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
}

/// Membership API
impl<NID, N> Membership<NID, N>
where
    N: Node,
    NID: NodeId,
{
    /// Returns an Iterator of all voter node ids. Learners are not included.
    pub fn voter_ids(&self) -> impl Iterator<Item = NID> {
        self.configs.as_joint().ids()
    }

    /// Returns an Iterator of all learner node ids. Voters are not included.
    pub fn learner_ids(&self) -> impl Iterator<Item = NID> + '_ {
        self.nodes.keys().filter(|x| !self.is_voter(x)).copied()
    }

    /// Returns an Iterator of all nodes(voters and learners).
    pub fn nodes(&self) -> impl Iterator<Item = (&NID, &N)> {
        self.nodes.iter()
    }

    /// Get a the node(either voter or learner) by node id.
    pub fn get_node(&self, node_id: &NID) -> Option<&N> {
        self.nodes.get(node_id)
    }

    /// Return if a node is a voter or learner, or not in this membership config at all.
    #[allow(dead_code)]
    pub(crate) fn get_node_role(&self, nid: &NID) -> Option<NodeRole> {
        if self.is_voter(nid) {
            Some(NodeRole::Voter)
        } else if self.contains(nid) {
            Some(NodeRole::Learner)
        } else {
            None
        }
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

    /// Returns if a voter or learner exists in this membership.
    pub(crate) fn contains(&self, node_id: &NID) -> bool {
        self.nodes.contains_key(node_id)
    }

    /// Returns reference to the joint config.
    ///
    /// Membership is defined by a joint of multiple configs.
    /// Each config is a vec of node-id.
    pub fn get_joint_config(&self) -> &Vec<BTreeSet<NID>> {
        &self.configs
    }
}

/// Quorum related API
impl<NID, N> Membership<NID, N>
where
    N: Node,
    NID: NodeId,
{
    /// Returns the next coherent membership to change to, while the expected final membership is
    /// `goal`.
    ///
    /// `retain` specifies whether to retain the removed voters as a learners, i.e., nodes that
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
        let config = Joint::from(self.configs.clone()).find_coherent(goal).children().clone();

        let mut nodes = self.nodes.clone();

        if !retain {
            let old_voter_ids = self.configs.as_joint().ids().collect::<BTreeSet<_>>();
            let new_voter_ids = config.as_joint().ids().collect::<BTreeSet<_>>();

            for node_id in old_voter_ids.difference(&new_voter_ids) {
                nodes.remove(node_id);
            }
        };

        Membership::new_with_nodes(config, nodes)
    }

    /// Apply a change-membership request and return a new instance.
    ///
    /// It ensures that the returned instance is valid.
    ///
    /// `retain` specifies whether to retain the removed voters as a learners, i.e., nodes that
    /// continue to receive log replication from the leader.
    pub(crate) fn change(
        mut self,
        change: ChangeMembers<NID, N>,
        retain: bool,
    ) -> Result<Self, ChangeMembershipError<NID>> {
        tracing::debug!(change = debug(&change), "{}", func_name!());

        let last = self.get_joint_config().last().unwrap();

        let new_membership = match change {
            ChangeMembers::AddVoter(add_voter_ids) => {
                let new_voter_ids = last.union(&add_voter_ids).copied().collect::<BTreeSet<_>>();
                self.next_coherent(new_voter_ids, retain)
            }
            ChangeMembers::RemoveVoter(remove_voter_ids) => {
                let new_voter_ids = last.difference(&remove_voter_ids).copied().collect::<BTreeSet<_>>();
                self.next_coherent(new_voter_ids, retain)
            }
            ChangeMembers::ReplaceAllVoters(all_voter_ids) => self.next_coherent(all_voter_ids, retain),
            ChangeMembers::AddNodes(add_nodes) => {
                // When adding nodes, do not override existing node
                for (node_id, node) in add_nodes.into_iter() {
                    self.nodes.entry(node_id).or_insert(node);
                }
                self
            }
            ChangeMembers::RemoveNodes(remove_node_ids) => {
                for node_id in remove_node_ids.iter() {
                    self.nodes.remove(node_id);
                }
                self
            }
            ChangeMembers::ReplaceAllNodes(all_nodes) => {
                self.nodes = all_nodes;
                self
            }
        };

        tracing::debug!(new_membership = display(&new_membership), "new membership");

        new_membership.ensure_valid()?;

        Ok(new_membership)
    }

    /// Build a QuorumSet from current joint config
    pub(crate) fn to_quorum_set(&self) -> Joint<NID, Vec<NID>, Vec<Vec<NID>>> {
        let mut qs = vec![];
        for c in self.get_joint_config().iter() {
            qs.push(c.iter().copied().collect::<Vec<_>>());
        }
        Joint::new(qs)
    }
}

#[cfg(test)]
mod tests {
    use maplit::btreemap;
    use maplit::btreeset;

    use crate::error::ChangeMembershipError;
    use crate::error::EmptyMembership;
    use crate::error::LearnerNotFound;
    use crate::ChangeMembers;
    use crate::Membership;

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
            let res = m().change(ChangeMembers::AddVoter(btreeset! {4}), true);
            assert_eq!(
                Err(ChangeMembershipError::LearnerNotFound(LearnerNotFound { node_id: 4 })),
                res
            );
        }

        // Add: ok
        {
            let res = m().change(ChangeMembers::AddVoter(btreeset! {3}), true);
            assert_eq!(
                Ok(Membership::<u64, ()> {
                    configs: vec![btreeset! {1,2}, btreeset! {1,2,3}],
                    nodes: btreemap! {1=>(),2=>(),3=>()}
                }),
                res
            );
        }

        // Remove: no such voter
        {
            let res = m().change(ChangeMembers::RemoveVoter(btreeset! {5}), true);
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
            let res = m().change(ChangeMembers::RemoveVoter(btreeset! {1,2}), true);
            assert_eq!(Err(ChangeMembershipError::EmptyMembership(EmptyMembership {})), res);
        }

        // Remove: OK retain
        {
            let res = m().change(ChangeMembers::RemoveVoter(btreeset! {1}), true);
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
            let res = m().change(ChangeMembers::RemoveVoter(btreeset! {1}), false);
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
            let res = mem.change(ChangeMembers::RemoveVoter(btreeset! {1}), false);
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

        // RemoveNodes: can not remove node for voter
        {
            let res = m().change(ChangeMembers::RemoveNodes(btreeset! {2}), false);
            assert_eq!(
                Err(ChangeMembershipError::LearnerNotFound(LearnerNotFound { node_id: 2 })),
                res
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
}
