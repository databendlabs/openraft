use core::fmt;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;

use openraft_macros::since;

use crate::errors::MembershipError;
use crate::impls::ChangeMembers;
use crate::membership::IntoNodes;
use crate::membership::RaftMembershipExt;
use crate::node::EmptyNode;
use crate::node::Node;
use crate::node::NodeId;

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

impl<NID> From<BTreeSet<NID>> for Membership<NID, ()>
where NID: NodeId
{
    fn from(node_ids: BTreeSet<NID>) -> Self {
        let nodes: BTreeMap<NID, ()> = node_ids.into_nodes();
        Membership::from(nodes)
    }
}

impl<NID> From<BTreeSet<NID>> for Membership<NID, EmptyNode>
where NID: NodeId
{
    fn from(node_ids: BTreeSet<NID>) -> Self {
        let nodes: BTreeMap<NID, EmptyNode> = node_ids.into_nodes();
        Membership::from(nodes)
    }
}

impl<NID, N> From<HashMap<NID, N>> for Membership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn from(nodes: HashMap<NID, N>) -> Self {
        let nodes: BTreeMap<NID, N> = nodes.into_nodes();
        Membership::from(nodes)
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

        let learner_ids = self.nodes.keys().filter(|id| self.is_learner(id));

        write!(f, ", learners:[")?;

        for (learner_cnt, learner_id) in learner_ids.enumerate() {
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
    /// `config` may hold an empty voter set, or no voter set at all. [`Raft::initialize()`] and
    /// [`Raft::append_membership()`] check that a membership has a usable quorum before they write
    /// it to the log.
    ///
    /// The `nodes` implements [`IntoNodes`] thus it can be `BTreeMap<NodeId, Node>` or
    /// `HashMap<NodeId,Node>` including all Voter and Learner nodes.
    ///
    /// [`Raft::initialize()`]: crate::Raft::initialize
    /// [`Raft::append_membership()`]: crate::Raft::append_membership
    pub fn new<T>(config: Vec<BTreeSet<NID>>, nodes: T) -> Result<Self, MembershipError<NID>>
    where T: IntoNodes<NID, N> {
        let m = Membership {
            configs: config,
            nodes: nodes.into_nodes(),
        };

        m.ensure_voters_have_nodes()?;
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

    /// Deprecated alias for [`Self::non_write_voter_ids()`].
    #[since(version = "0.10.0", change = "deprecated in favor of `non_write_voter_ids()`")]
    #[deprecated(since = "0.10.0", note = "use `non_write_voter_ids()` instead")]
    pub fn learner_ids(&self) -> impl Iterator<Item = NID> + '_ {
        self.non_write_voter_ids()
    }

    /// Returns known node IDs outside the write voter configurations, in ascending order.
    ///
    /// These nodes receive replicated logs without counting toward the write quorum.
    #[since(version = "0.10.0")]
    pub fn non_write_voter_ids(&self) -> impl Iterator<Item = NID> + '_ {
        self.nodes.keys().filter(|x| !self.is_write_voter(x)).cloned()
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

    /// Returns whether `node_id` belongs to this membership.
    pub(crate) fn contains(&self, node_id: &NID) -> bool {
        self.nodes.contains_key(node_id)
    }

    /// Returns whether `node_id` may start an election under this configuration.
    ///
    /// This majority-based membership lets every write voter campaign.
    pub(crate) fn is_campaigner(&self, node_id: &NID) -> bool {
        self.is_write_voter(node_id)
    }

    /// Returns whether `node_id` is a read voter in any component configuration.
    ///
    /// In a joint membership, belonging to any component is enough to be a read voter.
    /// A read quorum still requires a majority of every component.
    /// This membership uses the same voters for read and write quorums.
    pub(crate) fn is_read_voter(&self, node_id: &NID) -> bool {
        for c in self.configs.iter() {
            if c.contains(node_id) {
                return true;
            }
        }
        false
    }

    /// Returns whether `node_id` is a write voter in any component configuration.
    ///
    /// In a joint membership, belonging to any component is enough to be a write voter.
    /// A write quorum still requires a majority of every component.
    /// This membership uses the same voters for read and write quorums.
    pub(crate) fn is_write_voter(&self, node_id: &NID) -> bool {
        self.is_read_voter(node_id)
    }

    /// Returns whether `node_id` is a configured learner.
    ///
    /// Learners receive logs but have no campaign, read voter, or write voter role.
    /// Nodes absent from this membership are not learners.
    pub(crate) fn is_learner(&self, node_id: &NID) -> bool {
        if !self.contains(node_id) {
            return false;
        }

        let is_campaigner = self.is_campaigner(node_id);
        let is_read_voter = self.is_read_voter(node_id);
        let is_write_voter = self.is_write_voter(node_id);
        let is_voter = is_read_voter || is_write_voter;
        let has_role = is_campaigner || is_voter;
        !has_role
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

    // ---
    // Quorum related internal API
    // ---
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
    pub(crate) fn compute_target_membership(mut self, change: ChangeMembers<NID, N>) -> Membership<NID, N> {
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

    use crate::Membership;
    use crate::errors::NodeNotFound;
    use crate::errors::Operation;
    use crate::impls::ChangeMembers;
    use crate::membership::RaftMembershipEdit;
    use crate::membership::RaftMembershipExt;

    #[test]
    fn test_membership_ensure_voters_have_nodes() -> anyhow::Result<()> {
        let m = Membership::<u64, ()> {
            configs: vec![btreeset! {1,2}],
            nodes: btreemap! {1=>()},
        };
        let res = m.ensure_voters_have_nodes();
        assert_eq!(Err(NodeNotFound::new(2, Operation::None)), res);
        Ok(())
    }

    #[test]
    fn test_membership_change() -> anyhow::Result<()> {
        let m = || Membership::<u64, ()> {
            configs: vec![btreeset! {1,2}],
            nodes: btreemap! {1=>(),2=>(),3=>()},
        };

        // Add: no such learner; `next_membership()` rejects this result
        {
            let membership = m();
            let change = ChangeMembers::AddVoterIds(btreeset! {4});
            let res = change.apply(&membership);
            assert_eq!(
                Ok(Membership::<u64, ()> {
                    configs: vec![btreeset! {1,2,4}],
                    nodes: btreemap! {1=>(),2=>(),3=>()}
                }),
                res
            );
        }

        // Add: ok
        {
            let membership = m();
            let change = ChangeMembers::AddVoterIds(btreeset! {3});
            let res = change.apply(&membership);
            assert_eq!(
                Ok(Membership::<u64, ()> {
                    configs: vec![btreeset! {1,2,3}],
                    nodes: btreemap! {1=>(),2=>(),3=>()}
                }),
                res
            );
        }

        // AddVoters
        {
            let membership = m();
            let change = ChangeMembers::AddVoters(btreemap! {5=>()});
            let res = change.apply(&membership);
            assert_eq!(
                Ok(Membership::<u64, ()> {
                    configs: vec![btreeset! {1,2,5}],
                    nodes: btreemap! {1=>(),2=>(),3=>(),5=>()}
                }),
                res
            );
        }

        // Remove: no such voter
        {
            let membership = m();
            let change = ChangeMembers::RemoveVoters(btreeset! {5});
            let res = change.apply(&membership);
            assert_eq!(
                Ok(Membership::<u64, ()> {
                    configs: vec![btreeset! {1,2}],
                    nodes: btreemap! {1=>(),2=>(),3=>()}
                }),
                res
            );
        }

        // Remove: become empty; `next_membership()` rejects this result
        {
            let membership = m();
            let change = ChangeMembers::RemoveVoters(btreeset! {1,2});
            let res = change.apply(&membership);
            assert_eq!(
                Ok(Membership::<u64, ()> {
                    configs: vec![btreeset! {}],
                    nodes: btreemap! {1=>(),2=>(),3=>()}
                }),
                res
            );
        }

        // Remove: OK retain
        {
            let membership = m();
            let change = ChangeMembers::RemoveVoters(btreeset! {1});
            let res = change.apply(&membership);
            assert_eq!(
                Ok(Membership::<u64, ()> {
                    configs: vec![btreeset! {2}],
                    nodes: btreemap! {1=>(),2=>(),3=>()}
                }),
                res
            );
        }

        // Remove: OK, not retain; learner not removed
        {
            let membership = m();
            let change = ChangeMembers::RemoveVoters(btreeset! {1});
            let goal = change.apply(&membership)?;
            let res = m().drop_voters_demoted_to_learners(goal);
            assert_eq!(
                Membership::<u64, ()> {
                    configs: vec![btreeset! {2}],
                    nodes: btreemap! {2=>(),3=>()}
                },
                res
            );
        }

        // Remove: OK, not retain; learner removed
        {
            let mem = Membership::<u64, ()> {
                configs: vec![btreeset! {1,2}, btreeset! {2}],
                nodes: btreemap! {1=>(),2=>(),3=>()},
            };
            let change = ChangeMembers::RemoveVoters(btreeset! {1});
            let goal = change.apply(&mem)?;
            let res = mem.drop_voters_demoted_to_learners(goal);
            assert_eq!(
                Membership::<u64, ()> {
                    configs: vec![btreeset! {2}],
                    nodes: btreemap! {2=>(),3=>()}
                },
                res
            );
        }

        // Replace:
        {
            let membership = m();
            let change = ChangeMembers::ReplaceAllVoters(btreeset! {2});
            let goal = change.apply(&membership)?;
            let res = m().drop_voters_demoted_to_learners(goal);
            assert_eq!(
                Membership::<u64, ()> {
                    configs: vec![btreeset! {2}],
                    nodes: btreemap! {2=>(),3=>()}
                },
                res
            );
        }

        // AddNodes: existent voter
        {
            let membership = m();
            let change = ChangeMembers::AddNodes(btreemap! {2=>()});
            let res = change.apply(&membership);
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
            let membership = m();
            let change = ChangeMembers::AddNodes(btreemap! {3=>()});
            let res = change.apply(&membership);
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
            let membership = m();
            let change = ChangeMembers::AddNodes(btreemap! {4=>()});
            let res = change.apply(&membership);
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

            let membership = m();
            let change = ChangeMembers::SetNodes(btreemap! {3=>30, 4=>40});
            let res = change.apply(&membership);
            assert_eq!(
                Ok(Membership::<u64, u64> {
                    configs: vec![btreeset! {1,2}],
                    nodes: btreemap! {1=>1,2=>2,3=>30, 4=>40}
                }),
                res
            );
        }

        // RemoveNodes: cannot remove node for voter; `next_membership()` rejects this result
        {
            let membership = m();
            let change = ChangeMembers::RemoveNodes(btreeset! {2});
            let res = change.apply(&membership);
            assert_eq!(
                Ok(Membership::<u64, ()> {
                    configs: vec![btreeset! {1,2}],
                    nodes: btreemap! {1=>(),3=>()}
                }),
                res
            );
        }

        // RemoveNodes: Ok
        {
            let membership = m();
            let change = ChangeMembers::RemoveNodes(btreeset! {3});
            let res = change.apply(&membership);
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
            let membership = m();
            let change = ChangeMembers::ReplaceAllNodes(btreemap! {1=>(),2=>(),4=>()});
            let res = change.apply(&membership);
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
    #[test]
    fn test_membership_change_batch() -> anyhow::Result<()> {
        let m = || Membership::<u64, ()> {
            configs: vec![btreeset! {1,2,3}],
            nodes: btreemap! {1=>(),2=>(),3=>()},
        };

        let rm_2_add_5 = || {
            ChangeMembers::Batch(vec![
                ChangeMembers::RemoveVoters(btreeset! {2}),
                ChangeMembers::AddVoters(btreemap! {5=>()}),
            ])
        };

        let membership = m();
        let change = rm_2_add_5();
        let goal = change.apply(&membership)?;

        assert_eq!(goal, Membership::<u64, ()> {
            configs: vec![btreeset! {1,3,5}],
            nodes: btreemap! {1=>(),2=>(),3=>(),5=>()}
        });

        Ok(())
    }
}
