//! Membership change operations and types.
//!
//! This module defines the various types of membership changes that can be performed
//! on a Raft cluster, such as adding voters, removing voters, or replacing the entire membership.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;

use display_more::DisplaySliceExt;
use openraft_macros::since;

use crate::display_ext::DisplayBTreeMapDebugValueExt;
use crate::display_ext::DisplayBTreeSetExt;
use crate::node::Node;
use crate::node::NodeId;

/// Defines various actions to change the membership, including adding or removing learners or
/// voters.
#[since(
    version = "0.10.0",
    change = "replaced `C: RaftTypeConfig` with `NID: NodeId, N: Node`"
)]
#[since(version = "0.8.0")]
#[derive(Debug, Clone)]
#[derive(PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum ChangeMembers<NID, N>
where
    NID: NodeId,
    N: Node,
{
    /// Upgrade learners to voters.
    ///
    /// The learners have to present or [`error::LearnerNotFound`](`crate::error::LearnerNotFound`)
    /// error will be returned.
    AddVoterIds(BTreeSet<NID>),

    /// Add voters with corresponding nodes.
    AddVoters(BTreeMap<NID, N>),

    /// Remove voters, leave removed voters as learner or not.
    RemoveVoters(BTreeSet<NID>),

    /// Replace voter ids with a new set. The node of every new voter has to already be a learner.
    ReplaceAllVoters(BTreeSet<NID>),

    /// Add nodes to membership, as learners.
    ///
    /// it **WON'T** replace existing node.
    ///
    /// Prefer using this variant instead of `SetNodes` whenever possible, as `AddNodes` ensures
    /// safety, whereas incorrect usage of `SetNodes` can result in a brain split.
    /// See: [Update-Node](`crate::docs::cluster_control::dynamic_membership#update-node`)
    AddNodes(BTreeMap<NID, N>),

    /// Add or replace nodes in membership config.
    ///
    /// it **WILL** replace an existing node.
    ///
    /// Prefer using `AddNodes` instead of `SetNodes` whenever possible, as `AddNodes` ensures
    /// safety, whereas incorrect usage of `SetNodes` can result in a brain split.
    /// See: [Update-Node](`crate::docs::cluster_control::dynamic_membership#update-node`)
    SetNodes(BTreeMap<NID, N>),

    /// Remove nodes from membership.
    ///
    /// If a node is still a voter, it returns
    /// [`error::LearnerNotFound`](`crate::error::LearnerNotFound`) error.
    RemoveNodes(BTreeSet<NID>),

    /// Replace all nodes with a new set.
    ///
    /// Every voter has to have a corresponding node in the new
    /// set, otherwise it returns [`error::LearnerNotFound`](`crate::error::LearnerNotFound`) error.
    ReplaceAllNodes(BTreeMap<NID, N>),

    /// Apply multiple changes to membership config.
    ///
    /// The changes are applied in the order they are given.
    /// And it still finishes in a two-step joint config change.
    Batch(Vec<ChangeMembers<NID, N>>),
}

/// Convert a series of ids to a `Replace` operation.
impl<NID, N, I> From<I> for ChangeMembers<NID, N>
where
    NID: NodeId,
    N: Node,
    I: IntoIterator<Item = NID>,
{
    fn from(r: I) -> Self {
        let ids = r.into_iter().collect::<BTreeSet<NID>>();
        ChangeMembers::ReplaceAllVoters(ids)
    }
}

impl<NID, N> fmt::Display for ChangeMembers<NID, N>
where
    NID: NodeId,
    N: Node,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChangeMembers::AddVoterIds(ids) => {
                write!(f, "AddVoterIds({})", ids.display())
            }
            ChangeMembers::AddVoters(nodes) => {
                write!(f, "AddVoters({})", nodes.display())
            }
            ChangeMembers::RemoveVoters(ids) => {
                write!(f, "RemoveVoters({})", ids.display())
            }
            ChangeMembers::ReplaceAllVoters(ids) => {
                write!(f, "ReplaceAllVoters({})", ids.display())
            }
            ChangeMembers::AddNodes(nodes) => {
                write!(f, "AddNodes({})", nodes.display())
            }
            ChangeMembers::SetNodes(nodes) => {
                write!(f, "SetNodes({})", nodes.display())
            }
            ChangeMembers::RemoveNodes(ids) => {
                write!(f, "RemoveNodes({})", ids.display())
            }
            ChangeMembers::ReplaceAllNodes(nodes) => {
                write!(f, "ReplaceAllNodes({})", nodes.display())
            }
            ChangeMembers::Batch(changes) => {
                write!(f, "Batch({})", changes.as_slice().display_n(1024))
            }
        }
    }
}
