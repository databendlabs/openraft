use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;
use std::num::NonZeroU64;
use std::sync::Arc;

use openraft::Node;
use openraft::NodeId;
use openraft::errors::NodeNotFound;
use openraft::errors::Operation;
use openraft::membership::IntoNodes;
use openraft_macros::since;

use crate::InvalidThresholds;
use crate::ThresholdMembershipError;
use crate::quorum::Joint;
use crate::quorum::QuorumSet;
use crate::quorum::ThresholdQuorumSet;

/// The voters of a config and its write quorum size.
pub(crate) type ThresholdConfig<NID> = (Arc<BTreeSet<NID>>, NonZeroU64);

/// A membership whose quorums are any `read_size` or any `write_size` voters of a config.
///
/// A config holds a voter set and a `write_size`. Any `write_size` voters form a write quorum.
/// Its read quorum size is `voter_count + 1 - write_size`, the minimum that intersects every
/// write quorum. A candidate becomes leader only with votes from a read quorum, and an entry
/// commits once a write quorum stores it.
///
/// A membership holds two or more configs only during a joint change. Then a quorum must be a
/// quorum of every config. Rule i of
/// [`read_quorum_set()`](openraft::membership::RaftMembership::read_quorum_set) gives the voters
/// and the `read_size` of config i, and rule i of
/// [`write_quorum_set()`](openraft::membership::RaftMembership::write_quorum_set) gives its
/// `write_size`.
///
/// [`threshold_membership`](crate::docs) explains how to
/// choose the write size and how to change it.
///
/// # Examples
///
/// ```
/// use std::collections::BTreeSet;
///
/// use openraft::membership::{QuorumSet, RaftMembership};
/// use openraft_membership::ThresholdMembership;
///
/// // 5 voters: an election needs 4 votes, and a commit needs 2 copies.
/// let voters = BTreeSet::from([1, 2, 3, 4, 5]);
/// let membership = ThresholdMembership::<u64, ()>::new(voters.clone(), 2, voters).unwrap();
///
/// let read_accepts_three = membership.read_quorum_set().is_quorum([1, 2, 3].iter());
/// assert!(!read_accepts_three);
/// let write_accepts_two = membership.write_quorum_set().is_quorum([1, 2].iter());
/// assert!(write_accepts_two);
/// ```
#[since(version = "0.10.0", change = "removed change(); use RaftMembershipEdit::apply()")]
#[since(version = "0.10.0", change = "moved to the openraft-membership crate")]
#[since(version = "0.10.0", change = "removed error-source parameter E")]
#[since(version = "0.10.0")]
#[derive(Clone, PartialEq, Eq)]
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
    /// Derived read rules, cached for `RaftMembership::read_quorum_set()`. Serialization stores
    /// only the write rules and reconstructs this cache when decoding.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub(crate) read: Joint<ThresholdQuorumSet<NID>>,

    /// One rule per config, "any `write_size` of its voters". Rule i shares its voter set with
    /// rule i of `read`, so each voter set is stored once.
    pub(crate) write: Joint<ThresholdQuorumSet<NID>>,

    /// Every known node. A node in no config is a learner.
    pub(crate) nodes: BTreeMap<NID, N>,
}

impl<NID, N> fmt::Debug for ThresholdMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("ThresholdMembership");
        debug.field("read", &self.read);
        debug.field("write", &self.write);
        debug.field("nodes", &self.nodes);
        debug.finish()
    }
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

/// Makes every node a voter, with `write_size = n / 2 + 1` and `read_size = n + 1 - write_size`.
impl<NID, N> From<BTreeMap<NID, N>> for ThresholdMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn from(nodes: BTreeMap<NID, N>) -> Self {
        let voters: BTreeSet<NID> = nodes.keys().cloned().collect();
        if voters.is_empty() {
            return Self::default();
        }
        let voter_count = voters.len() as u64;
        let majority = NonZeroU64::MIN.saturating_add(voter_count / 2);

        let config = (Arc::new(voters), majority);
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
    /// Builds a membership with one config and the given write quorum size. The read quorum
    /// size is `voters.len() + 1 - write_size`.
    ///
    /// Every voter needs a node in `nodes`, and the other nodes in `nodes` are learners. `nodes`
    /// implements [`IntoNodes`], so it can be a `BTreeMap<NID, N>`, a `HashMap<NID, N>`, or a
    /// `BTreeSet<NID>` when `N` is `()` or [`EmptyNode`](openraft::EmptyNode).
    ///
    /// Returns [`ThresholdMembershipError::InvalidThresholds`] unless, with `n`
    /// voters, `1 <= write_size <= n`.
    /// Returns [`ThresholdMembershipError::NodeNotFound`] for a voter with no node.
    #[since(version = "0.10.0", change = "returns concrete ThresholdMembershipError")]
    pub fn new<T>(voters: BTreeSet<NID>, write_size: u64, nodes: T) -> Result<Self, ThresholdMembershipError<NID>>
    where T: IntoNodes<NID, N> {
        let voter_count = voters.len() as u64;
        let write_size = Self::check_write_size(voter_count, write_size)?;

        let config = (Arc::new(voters), write_size);
        let nodes = nodes.into_nodes();
        let membership = Self::from_configs([config], nodes);

        membership.ensure_voters_have_nodes()?;
        Ok(membership)
    }

    /// Returns the write size when `1 <= write_size <= voter_count`.
    pub(crate) fn check_write_size(voter_count: u64, write_size: u64) -> Result<NonZeroU64, InvalidThresholds> {
        let invalid = InvalidThresholds {
            voter_count,
            write_size,
        };

        let Some(non_zero_write_size) = NonZeroU64::new(write_size) else {
            return Err(invalid);
        };

        if write_size > voter_count {
            return Err(invalid);
        }

        Ok(non_zero_write_size)
    }

    /// Builds a membership from validated `configs`, the oldest first. Derives the read sizes;
    /// each read rule shares its voter set with the corresponding write rule.
    pub(crate) fn from_configs<I>(configs: I, nodes: BTreeMap<NID, N>) -> Self
    where I: IntoIterator<Item = ThresholdConfig<NID>> {
        let mut read_rules = Vec::new();
        let mut write_rules = Vec::new();
        for (voters, write_size) in configs {
            let voter_count = voters.len() as u64;
            let read_size = voter_count - write_size.get() + 1;
            let read_size = NonZeroU64::new(read_size).unwrap();
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

    fn ensure_voters_have_nodes(&self) -> Result<(), NodeNotFound<NID>> {
        for node_id in self.write.ids() {
            if !self.nodes.contains_key(&node_id) {
                return Err(NodeNotFound::new(node_id, Operation::None));
            }
        }
        Ok(())
    }

    /// Returns the configs that `read` and `write` describe.
    ///
    /// Returns an error message unless both hold the same number of rules, rule i of each holds
    /// the same voters, and each read size is the minimum that intersects its write quorums.
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

        let configs = Self::configs_from_write_rules(write)?;
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
            let expected_read_size = voter_count - write_rule.quorum_size() + 1;
            if read_rule.quorum_size() != expected_read_size {
                let message = format!(
                    "read_size={} must equal voter_count + 1 - write_size={expected_read_size}",
                    read_rule.quorum_size()
                );
                return Err(message);
            }
        }

        Ok(configs)
    }

    /// Checks the write sizes and returns one config per write rule.
    pub(crate) fn configs_from_write_rules(
        write: &Joint<ThresholdQuorumSet<NID>>,
    ) -> Result<Vec<ThresholdConfig<NID>>, String> {
        let mut configs = Vec::new();
        for rule in write.rules() {
            let voters = rule.id_set();
            let voter_count = voters.len() as u64;
            let size = Self::check_write_size(voter_count, rule.quorum_size());
            let write_size = size.map_err(|e| e.to_string())?;
            configs.push((voters.clone(), write_size));
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
#[serde(bound = "", deny_unknown_fields)]
struct ThresholdMembershipFields<NID, N>
where
    NID: NodeId,
    N: Node,
{
    write: Joint<ThresholdQuorumSet<NID>>,
    nodes: BTreeMap<NID, N>,
}

/// Checks the decoded write configs and voter metadata, then derives their read rules.
#[cfg(feature = "serde")]
impl<NID, N> TryFrom<ThresholdMembershipFields<NID, N>> for ThresholdMembership<NID, N>
where
    NID: NodeId,
    N: Node,
{
    type Error = String;

    fn try_from(fields: ThresholdMembershipFields<NID, N>) -> Result<Self, String> {
        let configs = Self::configs_from_write_rules(&fields.write)?;
        let membership = Self::from_configs(configs, fields.nodes);

        let voters_have_nodes = membership.ensure_voters_have_nodes();
        voters_have_nodes.map_err(|e| e.to_string())?;
        Ok(membership)
    }
}
