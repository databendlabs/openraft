use core::option::Option;
use std::collections::BTreeMap;
use std::collections::BTreeSet;

use maplit::btreemap;

use crate::error::MissingNodeInfo;
use crate::membership::NodeRole;
use crate::quorum::AsJoint;
use crate::quorum::FindCoherent;
use crate::quorum::Joint;
use crate::quorum::QuorumSet;
use crate::MessageSummary;
use crate::Node;
use crate::NodeId;

/// Convert other types into the internal data structure for node infos
pub trait IntoOptionNodes<NID: NodeId> {
    fn into_option_nodes(self) -> BTreeMap<NID, Option<Node>>;
}

impl<NID: NodeId> IntoOptionNodes<NID> for () {
    fn into_option_nodes(self) -> BTreeMap<NID, Option<Node>> {
        btreemap! {}
    }
}

impl<NID: NodeId> IntoOptionNodes<NID> for BTreeSet<NID> {
    fn into_option_nodes(self) -> BTreeMap<NID, Option<Node>> {
        self.into_iter().map(|node_id| (node_id, None)).collect()
    }
}

impl<NID: NodeId> IntoOptionNodes<NID> for BTreeMap<NID, Node> {
    fn into_option_nodes(self) -> BTreeMap<NID, Option<Node>> {
        self.into_iter().map(|(node_id, n)| (node_id, Some(n))).collect()
    }
}

impl<NID: NodeId> IntoOptionNodes<NID> for BTreeMap<NID, Option<Node>> {
    fn into_option_nodes(self) -> BTreeMap<NID, Option<Node>> {
        self
    }
}

/// The membership configuration of the cluster.
///
/// It could be a joint of one, two or more configs, i.e., a quorum is a node set that is superset of a majority of
/// every config.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct Membership<NID: NodeId> {
    /// Multi configs of members.
    ///
    /// AKA a joint config in original raft paper.
    configs: Vec<BTreeSet<NID>>,

    /// Additional info of all nodes, e.g., the connecting host and port.
    ///
    /// A node-id key that is in `nodes` but is not in `configs` is a **learner**.
    /// The values in this map must all be `Some` or `None`.
    nodes: BTreeMap<NID, Option<Node>>,
}

impl<NID: NodeId> TryFrom<BTreeMap<NID, Option<Node>>> for Membership<NID> {
    type Error = MissingNodeInfo<NID>;

    fn try_from(b: BTreeMap<NID, Option<Node>>) -> Result<Self, Self::Error> {
        let member_ids = b.keys().cloned().collect::<BTreeSet<NID>>();

        let membership = Membership::with_nodes(vec![member_ids], b)?;
        Ok(membership)
    }
}

impl<NID: NodeId> MessageSummary<Membership<NID>> for Membership<NID> {
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

                if let Some(n) = self.get_node(node_id) {
                    res.push(format!(":{{{}}}", n));
                }
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

            if let Some(n) = self.get_node(learner_id) {
                res.push(format!(":{{{}}}", n));
            }
        }
        res.push("]".to_string());
        res.join("")
    }
}

impl<NID: NodeId> Membership<NID> {
    /// Create a new Membership of multiple configs(joint) and optionally a set of learner node ids.
    ///
    /// A node id that is in `node_ids` but is not in `configs` is a **learner**.
    pub fn new(configs: Vec<BTreeSet<NID>>, node_ids: Option<BTreeSet<NID>>) -> Self {
        let voter_ids = configs.as_joint().ids().collect::<BTreeSet<_>>();

        let nodes = match node_ids {
            None => {
                btreemap! {}
            }
            Some(x) => x.into_option_nodes(),
        };

        let nodes = Self::extend_nodes(nodes, &voter_ids.into_option_nodes());

        Membership { configs, nodes }
    }

    /// Create a new Membership of multiple configs and optional node infos.
    ///
    /// The node infos `nodes` can be:
    /// - a simple `()`, if there are no non-member nodes and no node infos are provided,
    /// - `BTreeSet<NodeId>` to specify additional learner node ids without node infos provided,
    /// - `BTreeMap<NodeId, Node>` or `BTreeMap<NodeId, Option<Node>>` to specify learner nodes with node infos. Node
    ///   ids not in `configs` are learner node ids. In this case, every node id in `configs` has to present in `nodes`
    ///   or an error will be returned.
    pub(crate) fn with_nodes<T>(configs: Vec<BTreeSet<NID>>, nodes: T) -> Result<Self, MissingNodeInfo<NID>>
    where T: IntoOptionNodes<NID> {
        let nodes = nodes.into_option_nodes();

        for voter_id in configs.as_joint().ids() {
            if !nodes.contains_key(&voter_id) {
                return Err(MissingNodeInfo {
                    node_id: voter_id,
                    reason: format!("is not in cluster: {:?}", nodes.keys().cloned().collect::<Vec<_>>()),
                });
            }
        }

        let has_some = nodes.values().any(|x| x.is_some());
        if has_some {
            let first_none = nodes.iter().find(|(_node_id, v)| v.is_none());
            if let Some(first_none) = first_none {
                return Err(MissingNodeInfo {
                    node_id: *first_none.0,
                    reason: "is None".to_string(),
                });
            }
        }

        Ok(Membership { configs, nodes })
    }

    /// Extends nodes btreemap with another.
    ///
    /// Node that present in `old` will **NOT** be replaced because changing the address of a node potentially breaks
    /// consensus guarantee.
    pub(crate) fn extend_nodes(
        old: BTreeMap<NID, Option<Node>>,
        new: &BTreeMap<NID, Option<Node>>,
    ) -> BTreeMap<NID, Option<Node>> {
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

    pub(crate) fn add_learner(&self, node_id: NID, node: Option<Node>) -> Result<Self, MissingNodeInfo<NID>> {
        let configs = self.configs.clone();

        let nodes = Self::extend_nodes(self.nodes.clone(), &btreemap! {node_id=>node});

        let m = Self::with_nodes(configs, nodes)?;

        Ok(m)
    }
}

/// Membership API
impl<NID: NodeId> Membership<NID> {
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

    /// Returns an Iterator of all voter node ids. Learners are not included.
    pub(crate) fn voter_ids(&self) -> impl Iterator<Item = NID> {
        self.configs.as_joint().ids()
    }

    /// Returns an Iterator of all learner node ids. Voters are not included.
    #[allow(dead_code)]
    pub(crate) fn learner_ids(&self) -> impl Iterator<Item = NID> + '_ {
        self.nodes.keys().filter(|x| !self.is_voter(x)).copied()
    }

    /// Returns if a voter or learner exists in this membership.
    pub(crate) fn contains(&self, node_id: &NID) -> bool {
        self.nodes.contains_key(node_id)
    }

    /// Get a the node(either voter or learner) by node id.
    pub(crate) fn get_node(&self, node_id: &NID) -> Option<&Node> {
        let x = self.nodes.get(node_id)?;
        x.as_ref()
    }

    /// Returns an Iterator of all nodes(voters and learners).
    pub fn nodes(&self) -> impl Iterator<Item = (&NID, &Option<Node>)> {
        self.nodes.iter()
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
impl<NID: NodeId> Membership<NID> {
    /// Returns the next safe membership to change to while the expected final membership is `goal`.
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
    ///     let next = curr.next_step(goal);
    ///     change_membership(next);
    ///     curr = next;
    /// }
    /// ```
    pub(crate) fn next_safe<T>(&self, goal: T, turn_to_learner: bool) -> Result<Self, MissingNodeInfo<NID>>
    where T: IntoOptionNodes<NID> {
        let goal = goal.into_option_nodes();

        let goal_ids = goal.keys().cloned().collect::<BTreeSet<_>>();

        let config = Joint::from(self.configs.clone()).find_coherent(goal_ids).children().clone();

        let mut nodes = Self::extend_nodes(self.nodes.clone(), &goal);

        if !turn_to_learner {
            let old_voter_ids = self.configs.as_joint().ids().collect::<BTreeSet<_>>();
            let new_voter_ids = config.as_joint().ids().collect::<BTreeSet<_>>();

            for node_id in old_voter_ids.difference(&new_voter_ids) {
                nodes.remove(node_id);
            }
        };

        let m = Membership::with_nodes(config, nodes)?;
        Ok(m)
    }
}
