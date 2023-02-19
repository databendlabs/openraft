use std::collections::BTreeMap;
use std::collections::BTreeSet;

use maplit::btreemap;

use crate::membership::NodeRole;
use crate::node::Node;
use crate::quorum::AsJoint;
use crate::quorum::FindCoherent;
use crate::quorum::Joint;
use crate::quorum::QuorumSet;
use crate::MessageSummary;
use crate::NodeId;

/// Convert types into a map of `Node`.
pub trait IntoNodes<NID, N>
where
    N: Node,
    NID: NodeId,
{
    fn into_nodes(self) -> BTreeMap<NID, N>;
}

impl<NID, N> IntoNodes<NID, N> for ()
where
    N: Node,
    NID: NodeId,
{
    fn into_nodes(self) -> BTreeMap<NID, N> {
        btreemap! {}
    }
}

impl<NID, N> IntoNodes<NID, N> for BTreeSet<NID>
where
    N: Node,
    NID: NodeId,
{
    fn into_nodes(self) -> BTreeMap<NID, N> {
        self.into_iter().map(|node_id| (node_id, N::default())).collect()
    }
}

impl<NID, N> IntoNodes<NID, N> for Option<BTreeSet<NID>>
where
    N: Node,
    NID: NodeId,
{
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
        Membership::with_nodes(vec![member_ids], b)
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

                let n = self.get_node(node_id).unwrap();
                res.push(format!(":{{{:?}}}", n));
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

            let n = self.get_node(learner_id).unwrap();
            res.push(format!(":{{{:?}}}", n));
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
    ///
    /// Every node id in `configs` has to present in `nodes`. This is the only difference from
    /// [`Membership::new`].
    pub(crate) fn with_nodes<T>(configs: Vec<BTreeSet<NID>>, nodes: T) -> Self
    where T: IntoNodes<NID, N> {
        let nodes = nodes.into_nodes();

        if cfg!(debug_assertions) {
            for voter_id in configs.as_joint().ids() {
                assert!(
                    nodes.contains_key(&voter_id),
                    "nodes has to contain voter id {}",
                    voter_id
                );
            }
        }

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

    pub(crate) fn add_learner(&self, node_id: NID, node: N) -> Self {
        let configs = self.configs.clone();

        let nodes = Self::extend_nodes(self.nodes.clone(), &btreemap! {node_id=>node});

        Self::with_nodes(configs, nodes)
    }
}

/// Membership API
impl<NID, N> Membership<NID, N>
where
    N: Node,
    NID: NodeId,
{
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
    pub(crate) fn get_node(&self, node_id: &NID) -> Option<&N> {
        self.nodes.get(node_id)
    }

    /// Returns an Iterator of all nodes(voters and learners).
    pub fn nodes(&self) -> impl Iterator<Item = (&NID, &N)> {
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
impl<NID, N> Membership<NID, N>
where
    N: Node,
    NID: NodeId,
{
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
    pub(crate) fn next_safe<T>(&self, goal: T, removed_to_learner: bool) -> Self
    where T: IntoNodes<NID, N> {
        let goal = goal.into_nodes();

        let goal_ids = goal.keys().cloned().collect::<BTreeSet<_>>();

        let config = Joint::from(self.configs.clone()).find_coherent(goal_ids).children().clone();

        let mut nodes = Self::extend_nodes(self.nodes.clone(), &goal);

        if !removed_to_learner {
            let old_voter_ids = self.configs.as_joint().ids().collect::<BTreeSet<_>>();
            let new_voter_ids = config.as_joint().ids().collect::<BTreeSet<_>>();

            for node_id in old_voter_ids.difference(&new_voter_ids) {
                nodes.remove(node_id);
            }
        };

        Membership::with_nodes(config, nodes)
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
