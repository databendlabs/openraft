use core::fmt;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;

use openraft_macros::since;

use crate::errors::NodeNotFound;
use crate::membership::IntoNodes;
use crate::membership::RaftMembershipExt;
use crate::node::EmptyNode;
use crate::node::Node;
use crate::node::NodeId;

/// The membership configuration of the cluster.
///
/// It could be a joint of one, two or more configs, i.e., a quorum is a node set that is superset
/// of a majority of every config.
///
/// Deserialization checks the decoded membership as [`Membership::new()`] does: it fails when a
/// voter has no node, and accepts every membership that `new()` accepts.
#[since(version = "0.10.2", change = "deserialization rejects a voter without a node")]
#[since(
    version = "0.10.0",
    change = "replaced `C: RaftTypeConfig` with `NID: NodeId, N: Node`"
)]
#[derive(Clone, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize, serde::Serialize),
    serde(bound = "", try_from = "MembershipFields<NID, N>")
)]
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

impl<NID, N> fmt::Debug for Membership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("Membership");
        debug.field("configs", &self.configs);
        debug.field("nodes", &self.nodes);
        debug.finish()
    }
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
    #[since(version = "0.10.0", change = "returns NodeNotFound directly")]
    pub fn new<T>(config: Vec<BTreeSet<NID>>, nodes: T) -> Result<Self, NodeNotFound<NID>>
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

    /// Returns a reference to the joint config, the voter configurations of this membership.
    ///
    /// Each configuration is a `BTreeSet` of voter node IDs. Learners are in none of them. The
    /// returned `Vec` holds:
    ///
    /// - No configuration. [`Membership::default()`], the membership of a node that has not been
    ///   initialized, has none, and [`Self::new()`] and [`Self::new_with_defaults()`] accept an
    ///   empty `config`.
    /// - One configuration: a uniform config.
    /// - Two or more configurations: the membership is in joint consensus, and a quorum includes a
    ///   majority of every configuration.
    ///
    /// A configuration may also be an empty set. [`Raft::initialize()`],
    /// [`Raft::change_membership()`], and [`Raft::append_membership()`] reject a membership with no
    /// configuration or with an empty one before they write it to the log.
    ///
    /// [`Raft::initialize()`]: crate::Raft::initialize
    /// [`Raft::change_membership()`]: crate::Raft::change_membership
    /// [`Raft::append_membership()`]: crate::Raft::append_membership
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

    /// Create a new Membership from `configs` and `nodes` without checking that every voter has a
    /// node.
    ///
    /// [`Self::new()`] returns [`NodeNotFound`] for a voter that is not in `nodes`.
    /// [`Self::new_with_defaults()`] takes only node IDs and builds every node, voters included,
    /// with `N::default()`. This function does neither: a voter that is not in `nodes` has no node
    /// in the result, which breaks the [`RaftMembership`] invariant that every voter resolves
    /// through `get_node()`.
    ///
    /// [`RaftMembership`]: crate::membership::RaftMembership
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
}

/// The fields of an encoded [`Membership`], which decoding checks before it builds one.
///
/// It has the name, the fields, and the field order that a `Membership` is encoded with, so it
/// decodes every encoded `Membership`. It ignores unknown fields.
#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
#[serde(bound = "", rename = "Membership")]
struct MembershipFields<NID, N>
where
    NID: NodeId,
    N: Node,
{
    configs: Vec<BTreeSet<NID>>,
    nodes: BTreeMap<NID, N>,
}

/// Builds the decoded membership with [`Membership::new()`], which checks that every voter has a
/// node.
#[cfg(feature = "serde")]
impl<NID, N> TryFrom<MembershipFields<NID, N>> for Membership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    type Error = NodeNotFound<NID>;

    fn try_from(fields: MembershipFields<NID, N>) -> Result<Self, Self::Error> {
        Membership::new(fields.configs, fields.nodes)
    }
}

#[cfg(test)]
mod tests {
    use maplit::btreemap;
    use maplit::btreeset;

    use crate::Membership;
    use crate::errors::NodeNotFound;
    use crate::errors::Operation;
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
}
