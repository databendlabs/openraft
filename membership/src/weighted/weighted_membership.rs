use std::collections::BTreeMap;
use std::fmt;

use openraft::Node;
use openraft::NodeId;
use openraft::errors::MembershipError;
use openraft::membership::IntoNodes;
use openraft::membership::NodeRef;
use openraft::membership::NodeRole;
use openraft::membership::RaftMembership;
use openraft_macros::since;

use crate::config::Config;
use crate::errors::InvalidWeights;
use crate::quorum::Joint;
use crate::quorum::WeightedQuorumSet;

/// A membership whose quorums are the voter sets that weigh at least a threshold.
///
/// A config gives each voter a positive weight and sets a `write_weight`. A set of voters is a
/// write quorum when its voters weigh at least `write_weight` together, and a read quorum when
/// they weigh at least `read_weight = total_weight - write_weight + 1`. A read quorum and a write
/// quorum together weigh more than all voters, so they share a voter. A candidate becomes leader
/// only with votes from a read quorum, and an entry commits once a write quorum stores it.
///
/// A threshold counts weight, not voters, so the number of copies of a committed entry depends on
/// which voters store it. With weights `{1:3, 2:1, 3:1, 4:1}` and `write_weight = 3`, an entry
/// commits once voter 1 alone stores it, or once voters 2, 3 and 4 all store it. A heavy voter
/// thus lets an entry commit with few copies, and its failure removes much of the weight that
/// elections and commits need.
///
/// With weight 1 for every voter, `write_weight` is the write quorum size, and the quorums are
/// those of a [`ThresholdMembership`](crate::ThresholdMembership) with that write size.
///
/// OpenRaft changes a config directly, without a joint config, only when every read quorum of each
/// config is proven to intersect every write quorum of the other. The check proves it only when
/// both memberships hold a config with the same weights and the same `write_weight`. With the same
/// weights, raising `write_weight` lowers `read_weight`, so an old write quorum may miss a new read
/// quorum, and lowering it causes the reverse problem. Every change of the weights or of
/// `write_weight` therefore goes through a joint config.
///
/// A membership holds two or more configs only during a joint change. Then a quorum must be a
/// quorum of every config. Rule i of
/// [`read_quorum_set()`](openraft::membership::RaftMembership::read_quorum_set) and rule i of
/// [`write_quorum_set()`](openraft::membership::RaftMembership::write_quorum_set) belong to
/// config i and share its weights.
///
/// [`weighted_membership`](crate::docs::weighted) explains how to choose the weights and how to
/// change them.
///
/// # Node roles
///
/// [`with_roles()`](Self::with_roles) and [`WeightedChange`](crate::WeightedChange) set node roles.
/// The [shared role guide](crate::docs) explains defaults, leadership transfers and joint changes.
///
/// The weights alone decide which nodes are voters. Both rules of a config weigh the same voters,
/// so every voter is a read voter and a write voter. Every voter therefore needs
/// [`NodeRole::ReceiveLogs`], or a write quorum that holds the voter never commits.
///
/// # Examples
///
/// ```
/// use std::collections::BTreeMap;
/// use std::collections::BTreeSet;
///
/// use openraft::membership::{QuorumSet, RaftMembership};
/// use openraft_membership::WeightedMembership;
///
/// // The total weight is 6, so write weight 3 derives read weight 4.
/// let weights = BTreeMap::from([(1, 3), (2, 1), (3, 1), (4, 1)]);
/// let nodes = BTreeSet::from([1, 2, 3, 4]);
/// let membership = WeightedMembership::<u64, ()>::new(weights, 3, nodes).unwrap();
///
/// // Voter 1 alone stores enough weight to commit, but voters 2 and 3 do not.
/// let write_accepts_one = membership.write_quorum_set().is_quorum([1].iter());
/// assert!(write_accepts_one);
/// let write_accepts_two_three = membership.write_quorum_set().is_quorum([2, 3].iter());
/// assert!(!write_accepts_two_three);
///
/// // Every election needs voter 1: the other voters weigh only 3 together.
/// let read_accepts_others = membership.read_quorum_set().is_quorum([2, 3, 4].iter());
/// assert!(!read_accepts_others);
/// ```
#[since(version = "0.10.0")]
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub struct WeightedMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    pub(crate) configs: Vec<Config<NID, N, WeightedQuorumSet<NID>>>,
}

/// Written by hand: `derive(Default)` would require `NID: Default` and `N: Default`.
impl<NID, N> Default for WeightedMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn default() -> Self {
        Self { configs: Vec::new() }
    }
}

/// Makes every node a voter with weight 1, with `write_weight = n / 2 + 1` and
/// `read_weight = n - write_weight + 1`. The quorums are those of the
/// [`ThresholdMembership`](crate::ThresholdMembership) converted from the same map.
impl<NID, N> From<BTreeMap<NID, N>> for WeightedMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn from(nodes: BTreeMap<NID, N>) -> Self {
        if nodes.is_empty() {
            return Self::default();
        }

        let mut weights = BTreeMap::new();
        for node_id in nodes.keys() {
            weights.insert(node_id.clone(), 1);
        }
        let voter_count = nodes.len() as u64;
        let majority = voter_count / 2 + 1;

        let membership = Self::new(weights, majority, nodes);
        membership.expect("unit weights and complete node metadata form a valid config")
    }
}

impl<NID, N> fmt::Display for WeightedMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{configs: {:?}}}", self.configs)
    }
}

impl<NID, N> WeightedMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Builds a membership with one config that gives each voter of `weights` its weight and
    /// requires `write_weight` for a write quorum. The read weight is
    /// `total_weight - write_weight + 1`.
    ///
    /// Every voter needs a node in `nodes`, and the other nodes in `nodes` are learners. `nodes`
    /// implements [`IntoNodes`], so it can be a `BTreeMap<NID, N>`, a `HashMap<NID, N>`, or a
    /// `BTreeSet<NID>` when `N` is `()` or [`EmptyNode`](openraft::EmptyNode).
    ///
    /// Returns [`MembershipError::UserDefined`] for a weight of 0, for weights that
    /// sum to more than `u64::MAX`, and unless `1 <= write_weight <= total_weight`.
    /// Returns [`MembershipError::NodeNotFound`] for a voter with no node.
    pub fn new<T>(
        weights: BTreeMap<NID, u64>,
        write_weight: u64,
        nodes: T,
    ) -> Result<Self, MembershipError<NID, InvalidWeights<NID>>>
    where
        T: IntoNodes<NID, N>,
    {
        let (read, write) = Self::rules_from_weights(weights, write_weight)?;

        let nodes = nodes.into_nodes();
        let config = Config::new(read, write, BTreeMap::new(), nodes)?;
        Ok(Self { configs: vec![config] })
    }

    /// Returns this membership with each node of `roles` set to its role there in the last config,
    /// the only config of a membership from [`new()`](Self::new). The other nodes keep their roles.
    ///
    /// Returns [`MembershipError::NodeNotFound`] for a node that this membership does not
    /// know, and [`MembershipError::WriteVoterWithoutReceiveLogs`] for a write voter
    /// without [`NodeRole::ReceiveLogs`].
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeMap;
    /// use std::collections::BTreeSet;
    ///
    /// use openraft::membership::{NodeRef, NodeRole, RaftMembership};
    /// use openraft_membership::WeightedMembership;
    ///
    /// // Three voters of weight 1; any two of them form a quorum.
    /// let weights = BTreeMap::from([(1, 1), (2, 1), (3, 1)]);
    /// let nodes = BTreeSet::from([1, 2, 3]);
    /// let membership = WeightedMembership::<u64, ()>::new(weights, 2, nodes).unwrap();
    ///
    /// // Voter 3 stores logs and votes, but never starts an election on its own.
    /// let roles = BTreeMap::from([(3, NodeRole::ReceiveLogs)]);
    /// let membership = membership.with_roles(roles).unwrap();
    ///
    /// let got = membership.get_node(&3);
    /// let want = NodeRef {
    ///     node: &(),
    ///     role: NodeRole::ReceiveLogs,
    ///     is_read_voter: true,
    ///     is_write_voter: true,
    /// };
    /// assert_eq!(Some(want), got);
    /// ```
    pub fn with_roles(self, roles: BTreeMap<NID, NodeRole>) -> Result<Self, MembershipError<NID, InvalidWeights<NID>>> {
        let configs = Config::with_roles(self.configs, roles)?;
        Ok(Self { configs })
    }

    /// Returns the read rule and the write rule of a config with `weights` and `write_weight`.
    /// Both rules hold `weights`, and the read weight is `total_weight - write_weight + 1`.
    ///
    /// Returns [`InvalidWeights::ZeroWeight`] for the first voter with weight 0,
    /// [`InvalidWeights::TotalWeightOverflow`] when the weights sum to more than `u64::MAX`, and
    /// [`InvalidWeights::WriteWeightOutOfRange`] unless `1 <= write_weight <= total_weight`.
    pub(crate) fn rules_from_weights(
        weights: BTreeMap<NID, u64>,
        write_weight: u64,
    ) -> Result<(WeightedQuorumSet<NID>, WeightedQuorumSet<NID>), InvalidWeights<NID>> {
        let mut total_weight = 0u64;
        for (node_id, weight) in weights.iter() {
            if *weight == 0 {
                let node_id = node_id.clone();
                return Err(InvalidWeights::ZeroWeight { node_id });
            }
            let sum = total_weight.checked_add(*weight);
            total_weight = sum.ok_or(InvalidWeights::TotalWeightOverflow)?;
        }

        let out_of_range = InvalidWeights::WriteWeightOutOfRange {
            write_weight,
            total_weight,
        };
        let write_fits = (1..=total_weight).contains(&write_weight);
        if !write_fits {
            return Err(out_of_range);
        }

        // `1 <= write_weight <= total_weight`, so `read_weight` lies in `1..=total_weight`, and
        // computing it does not overflow.
        let read_weight = total_weight - write_weight + 1;

        let read = WeightedQuorumSet::new(read_weight, weights.clone())?;
        let write = WeightedQuorumSet::new(write_weight, weights)?;
        Ok((read, write))
    }
}

impl<NID, N> RaftMembership for WeightedMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    type NodeId = NID;
    type Node = N;
    type QuorumSet = Joint<WeightedQuorumSet<NID>>;

    fn read_quorum_set(&self) -> Self::QuorumSet {
        Config::read_quorum_set(&self.configs)
    }

    fn write_quorum_set(&self) -> Self::QuorumSet {
        Config::write_quorum_set(&self.configs)
    }

    fn supports_direct_transition_to(&self, other: &Self) -> bool {
        Config::supports_direct_transition(&self.configs, &other.configs)
    }

    fn get_node(&self, node_id: &Self::NodeId) -> Option<NodeRef<'_, Self::Node>> {
        Config::get_node(&self.configs, node_id)
    }

    type Iter = std::vec::IntoIter<NID>;

    fn node_ids(&self) -> Self::Iter {
        Config::node_ids(&self.configs)
    }

    fn joint_with(&self, goal: &Self) -> Self {
        let configs = Config::joint(&self.configs, &goal.configs);
        Self { configs }
    }
}
