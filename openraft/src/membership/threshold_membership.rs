use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;
use std::num::NonZeroU64;
use std::sync::Arc;

use openraft_macros::since;

use crate::Node;
use crate::NodeId;
use crate::errors::MembershipError;
use crate::membership::IntoNodes;
use crate::membership::InvalidThresholds;
use crate::membership::RaftMembershipExt;
use crate::quorum::Joint;
use crate::quorum::ThresholdQuorumSet;

/// The voters of a config, and how many of them a read quorum and a write quorum must hold.
pub(crate) type ThresholdConfig<NID> = (Arc<BTreeSet<NID>>, NonZeroU64, NonZeroU64);

/// A membership whose quorums are any `read_size` or any `write_size` voters of a config.
///
/// A config holds a voter set and two sizes. Any `read_size` of its voters form a read quorum,
/// and any `write_size` of them form a write quorum. A candidate becomes leader only with votes
/// from a read quorum, and an entry commits once a write quorum stores it. `read_size +
/// write_size` exceeds the number of voters, so every read quorum intersects every write quorum.
///
/// A membership holds two or more configs only during a joint change. Then a quorum must be a
/// quorum of every config. Rule i of
/// [`read_quorum_set()`](crate::membership::RaftMembership::read_quorum_set) gives the voters and
/// the `read_size` of config i, and rule i of
/// [`write_quorum_set()`](crate::membership::RaftMembership::write_quorum_set) gives its
/// `write_size`.
///
/// # Examples
///
/// ```
/// use std::collections::BTreeSet;
///
/// use openraft::membership::{QuorumSet, RaftMembership, ThresholdMembership};
///
/// // 5 voters: an election needs 4 votes, and a commit needs 2 copies.
/// let voters = BTreeSet::from([1, 2, 3, 4, 5]);
/// let membership = ThresholdMembership::<u64, ()>::new(voters.clone(), 4, 2, voters).unwrap();
///
/// let read_accepts_three = membership.read_quorum_set().is_quorum([1, 2, 3].iter());
/// assert!(!read_accepts_three);
/// let write_accepts_two = membership.write_quorum_set().is_quorum([1, 2].iter());
/// assert!(write_accepts_two);
/// ```
#[since(version = "0.10.0")]
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize, serde::Serialize),
    serde(bound = "", try_from = "ThresholdMembershipFields<NID, N>")
)]
pub struct ThresholdMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// One rule per config, "any `read_size` of its voters": the oldest config first, the target
    /// last. There are two or more rules only during a joint change.
    pub(crate) read: Joint<ThresholdQuorumSet<NID>>,

    /// One rule per config, "any `write_size` of its voters". Rule i shares its voter set with
    /// rule i of `read`, so each voter set is stored once.
    pub(crate) write: Joint<ThresholdQuorumSet<NID>>,

    /// Every known node. A node in no config is a learner.
    pub(crate) nodes: BTreeMap<NID, N>,
}

/// Written by hand: `derive(Default)` would require `NID: Default` and `N: Default`.
impl<NID, N> Default for ThresholdMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn default() -> Self {
        Self {
            read: Joint::default(),
            write: Joint::default(),
            nodes: BTreeMap::new(),
        }
    }
}

/// Makes every node a voter, with `read_size = write_size = n / 2 + 1` for `n` nodes. These are
/// the majority quorums of [`Membership`](crate::Membership).
impl<NID, N> From<BTreeMap<NID, N>> for ThresholdMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn from(nodes: BTreeMap<NID, N>) -> Self {
        let voters: BTreeSet<NID> = nodes.keys().cloned().collect();
        let voter_count = voters.len() as u64;
        let majority = NonZeroU64::MIN.saturating_add(voter_count / 2);

        let config = (Arc::new(voters), majority, majority);
        Self::from_configs([config], nodes)
    }
}

/// Prints each config as its voters and its two sizes, followed by the learners:
/// `{voters:[{1:(),2:(),3:()} read=2 write=2], learners:[4:()]}`.
impl<NID, N> fmt::Display for ThresholdMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{voters:[")?;

        for (i, (read, write)) in self.configs().enumerate() {
            if i > 0 {
                write!(f, ",")?;
            }

            write!(f, "{{")?;
            for (j, voter_id) in read.id_set().iter().enumerate() {
                if j > 0 {
                    write!(f, ",")?;
                }
                self.fmt_node(f, voter_id)?;
            }
            write!(f, "}} read={} write={}", read.quorum_size(), write.quorum_size())?;
        }

        write!(f, "], learners:[")?;

        let learner_ids = self.nodes.keys().filter(|id| !self.is_voter(id));
        for (i, learner_id) in learner_ids.enumerate() {
            if i > 0 {
                write!(f, ",")?;
            }
            self.fmt_node(f, learner_id)?;
        }

        write!(f, "]}}")
    }
}

impl<NID, N> ThresholdMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Builds a membership with one config: any `read_size` of `voters` form a read quorum, and
    /// any `write_size` of them form a write quorum.
    ///
    /// Every voter needs a node in `nodes`, and the other nodes in `nodes` are learners. `nodes`
    /// implements [`IntoNodes`], so it can be a `BTreeMap<NID, N>`, a `HashMap<NID, N>`, or a
    /// `BTreeSet<NID>` when `N` is `()` or [`EmptyNode`](crate::EmptyNode).
    ///
    /// Returns [`InvalidThresholds`] inside [`MembershipError::UserDefined`] unless, with `n`
    /// voters, `1 <= read_size <= n`, `1 <= write_size <= n` and `read_size + write_size > n`.
    /// The last condition holds exactly when every read quorum intersects every write quorum.
    /// Returns [`MembershipError::NodeNotFound`] for a voter with no node.
    pub fn new<T>(
        voters: BTreeSet<NID>,
        read_size: u64,
        write_size: u64,
        nodes: T,
    ) -> Result<Self, MembershipError<NID>>
    where
        T: IntoNodes<NID, N>,
    {
        let voter_count = voters.len() as u64;
        let (read_size, write_size) = Self::check_sizes(voter_count, read_size, write_size)?;

        let config = (Arc::new(voters), read_size, write_size);
        let nodes = nodes.into_nodes();
        let membership = Self::from_configs([config], nodes);

        membership.ensure_voters_have_nodes()?;
        Ok(membership)
    }

    /// Returns the two sizes when they give valid quorums for `voter_count` voters:
    /// `1 <= read_size <= voter_count`, `1 <= write_size <= voter_count` and
    /// `read_size + write_size > voter_count`.
    pub(crate) fn check_sizes(
        voter_count: u64,
        read_size: u64,
        write_size: u64,
    ) -> Result<(NonZeroU64, NonZeroU64), InvalidThresholds> {
        let invalid = InvalidThresholds {
            voter_count,
            read_size,
            write_size,
        };

        let Some(non_zero_read_size) = NonZeroU64::new(read_size) else {
            return Err(invalid);
        };
        let Some(non_zero_write_size) = NonZeroU64::new(write_size) else {
            return Err(invalid);
        };

        let sizes_fit = read_size <= voter_count && write_size <= voter_count;
        let size_sum = read_size.saturating_add(write_size);
        let quorums_intersect = size_sum > voter_count;
        if !sizes_fit || !quorums_intersect {
            return Err(invalid);
        }

        Ok((non_zero_read_size, non_zero_write_size))
    }

    /// Builds a membership from `configs`, the oldest first, without checking their sizes. The
    /// read rule and the write rule of a config share its voter set.
    pub(crate) fn from_configs<I>(configs: I, nodes: BTreeMap<NID, N>) -> Self
    where I: IntoIterator<Item = ThresholdConfig<NID>> {
        let mut read_rules = Vec::new();
        let mut write_rules = Vec::new();
        for (voters, read_size, write_size) in configs {
            let read_rule = ThresholdQuorumSet::new(read_size, voters.clone());
            let write_rule = ThresholdQuorumSet::new(write_size, voters);
            read_rules.push(read_rule);
            write_rules.push(write_rule);
        }

        Self {
            read: Joint::new(read_rules),
            write: Joint::new(write_rules),
            nodes,
        }
    }

    /// Returns the configs that `read` and `write` describe: config i takes the voters and the
    /// size of rule i of `read`, and the size of rule i of `write`.
    ///
    /// Returns an error message unless both hold the same number of rules, rule i of each holds
    /// the same voters, and the sizes of each config pass [`check_sizes()`](Self::check_sizes).
    pub(crate) fn configs_from_rules(
        read: &Joint<ThresholdQuorumSet<NID>>,
        write: &Joint<ThresholdQuorumSet<NID>>,
    ) -> Result<Vec<ThresholdConfig<NID>>, String> {
        let read_rules = read.rules();
        let write_rules = write.rules();
        if read_rules.len() != write_rules.len() {
            let message = format!("{} read rules but {} write rules", read_rules.len(), write_rules.len());
            return Err(message);
        }

        let mut configs = Vec::new();
        for (read_rule, write_rule) in read_rules.iter().zip(write_rules) {
            let voters = read_rule.id_set();
            if voters != write_rule.id_set() {
                let message = format!(
                    "a read rule holds the voters {:?}, but its write rule holds {:?}",
                    voters,
                    write_rule.id_set()
                );
                return Err(message);
            }

            let voter_count = voters.len() as u64;
            let sizes = Self::check_sizes(voter_count, read_rule.quorum_size(), write_rule.quorum_size());
            let (read_size, write_size) = sizes.map_err(|e| e.to_string())?;
            configs.push((voters.clone(), read_size, write_size));
        }

        Ok(configs)
    }

    /// Returns the read rule and the write rule of each config, the oldest config first.
    pub(crate) fn configs(&self) -> impl Iterator<Item = (&ThresholdQuorumSet<NID>, &ThresholdQuorumSet<NID>)> {
        let read_rules = self.read.rules().iter();
        let write_rules = self.write.rules().iter();
        read_rules.zip(write_rules)
    }

    /// Returns whether `node_id` is a voter of some config.
    pub(crate) fn is_voter(&self, node_id: &NID) -> bool {
        for rule in self.read.rules() {
            if rule.id_set().contains(node_id) {
                return true;
            }
        }
        false
    }

    /// Formats one node as `<node_id>:<node>`, or `<node_id>:None` if this membership has no such
    /// node.
    fn fmt_node(&self, f: &mut fmt::Formatter<'_>, node_id: &NID) -> fmt::Result {
        write!(f, "{node_id}:")?;
        match self.nodes.get(node_id) {
            Some(n) => write!(f, "{n:?}"),
            None => write!(f, "None"),
        }
    }
}

/// The fields of an encoded [`ThresholdMembership`], which decoding checks before it builds one.
#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
#[serde(bound = "")]
struct ThresholdMembershipFields<NID, N>
where
    NID: NodeId,
    N: Node,
{
    read: Joint<ThresholdQuorumSet<NID>>,
    write: Joint<ThresholdQuorumSet<NID>>,
    nodes: BTreeMap<NID, N>,
}

/// Checks the decoded configs with `ThresholdMembership::configs_from_rules()`, and checks that
/// every voter has a node. Then it rebuilds the membership, so that the two rules of a config share
/// one voter set again.
#[cfg(feature = "serde")]
impl<NID, N> TryFrom<ThresholdMembershipFields<NID, N>> for ThresholdMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    type Error = String;

    fn try_from(fields: ThresholdMembershipFields<NID, N>) -> Result<Self, String> {
        let configs = Self::configs_from_rules(&fields.read, &fields.write)?;
        let membership = Self::from_configs(configs, fields.nodes);

        let voters_have_nodes = membership.ensure_voters_have_nodes();
        voters_have_nodes.map_err(|e| e.to_string())?;
        Ok(membership)
    }
}
