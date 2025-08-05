use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::RaftTypeConfig;

/// Defines various actions to change the membership, including adding or removing learners or
/// voters.
#[derive(Debug, Clone)]
#[derive(PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
pub enum ChangeMembers<C>
where C: RaftTypeConfig
{
    /// Upgrade learners to voters.
    ///
    /// The learners have to present or [`error::LearnerNotFound`](`crate::error::LearnerNotFound`)
    /// error will be returned.
    AddVoterIds(BTreeSet<C::NodeId>),

    /// Add voters with corresponding nodes.
    AddVoters(BTreeMap<C::NodeId, C::Node>),

    /// Remove voters, leave removed voters as learner or not.
    RemoveVoters(BTreeSet<C::NodeId>),

    /// Replace voter ids with a new set. The node of every new voter has to already be a learner.
    ReplaceAllVoters(BTreeSet<C::NodeId>),

    /// Add nodes to membership, as learners.
    ///
    /// it **WON'T** replace existing node.
    ///
    /// Prefer using this variant instead of `SetNodes` whenever possible, as `AddNodes` ensures
    /// safety, whereas incorrect usage of `SetNodes` can result in a brain split.
    /// See: [Update-Node](`crate::docs::cluster_control::dynamic_membership#update-node`)
    AddNodes(BTreeMap<C::NodeId, C::Node>),

    /// Add or replace nodes in membership config.
    ///
    /// it **WILL** replace an existing node.
    ///
    /// Prefer using `AddNodes` instead of `SetNodes` whenever possible, as `AddNodes` ensures
    /// safety, whereas incorrect usage of `SetNodes` can result in a brain split.
    /// See: [Update-Node](`crate::docs::cluster_control::dynamic_membership#update-node`)
    SetNodes(BTreeMap<C::NodeId, C::Node>),

    /// Remove nodes from membership.
    ///
    /// If a node is still a voter, it returns
    /// [`error::LearnerNotFound`](`crate::error::LearnerNotFound`) error.
    RemoveNodes(BTreeSet<C::NodeId>),

    /// Replace all nodes with a new set.
    ///
    /// Every voter has to have a corresponding node in the new
    /// set, otherwise it returns [`error::LearnerNotFound`](`crate::error::LearnerNotFound`) error.
    ReplaceAllNodes(BTreeMap<C::NodeId, C::Node>),

    /// Apply multiple changes to membership config.
    ///
    /// The changes are applied in the order they are given.
    /// And it still finishes in a two-step joint config change.
    Batch(Vec<ChangeMembers<C>>),
}

/// Convert a series of ids to a `Replace` operation.
impl<C, I> From<I> for ChangeMembers<C>
where
    C: RaftTypeConfig,
    I: IntoIterator<Item = C::NodeId>,
{
    fn from(r: I) -> Self {
        let ids = r.into_iter().collect::<BTreeSet<C::NodeId>>();
        ChangeMembers::ReplaceAllVoters(ids)
    }
}
