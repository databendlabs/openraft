//! Membership change operations and types.
//!
//! This module defines the various types of membership changes that can be performed
//! on a Raft cluster, such as adding voters, removing voters, or replacing the entire membership.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::convert::Infallible;
use std::fmt;

use display_more::DisplayBTreeSetExt;
use display_more::DisplaySliceExt;
use openraft_macros::since;

use crate::Membership;
use crate::display_ext::DisplayBTreeMapDebugValueExt;
use crate::errors::EmptyMembership;
use crate::errors::MembershipError;
use crate::membership::RaftMembershipEdit;
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
    #[since(version = "0.10.0")]
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

impl<NID, N> RaftMembershipEdit<Membership<NID, N>> for ChangeMembers<NID, N>
where
    NID: NodeId,
    N: Node,
{
    type Error = Infallible;

    fn apply(self, membership: &Membership<NID, N>) -> Result<Membership<NID, N>, MembershipError<NID, Self::Error>> {
        tracing::debug!("{}: change: {:?}", func_name!(), self);

        let mut target = membership.clone();
        let mut pending = vec![self];
        while let Some(change) = pending.pop() {
            let configs = target.get_joint_config();
            let last = configs.last();
            let last = last.cloned().unwrap_or_default();

            let new_voter_ids = match change {
                ChangeMembers::AddVoterIds(add_voter_ids) => {
                    let voter_ids = last.union(&add_voter_ids);
                    let voter_ids = voter_ids.cloned().collect();
                    Some(voter_ids)
                }
                ChangeMembers::AddVoters(add_voters) => {
                    target.nodes = Membership::extend_nodes(target.nodes, &add_voters);
                    let node_ids = add_voters.keys();
                    let add_voter_ids = node_ids.cloned().collect::<BTreeSet<_>>();
                    let voter_ids = last.union(&add_voter_ids);
                    let voter_ids = voter_ids.cloned().collect();
                    Some(voter_ids)
                }
                ChangeMembers::RemoveVoters(remove_voter_ids) => {
                    let voter_ids = last.difference(&remove_voter_ids);
                    let voter_ids = voter_ids.cloned().collect();
                    Some(voter_ids)
                }
                ChangeMembers::ReplaceAllVoters(all_voter_ids) => Some(all_voter_ids),
                ChangeMembers::AddNodes(add_nodes) => {
                    for (node_id, node) in add_nodes {
                        target.nodes.entry(node_id).or_insert(node);
                    }
                    None
                }
                ChangeMembers::SetNodes(set_nodes) => {
                    for (node_id, node) in set_nodes {
                        target.nodes.insert(node_id, node);
                    }
                    None
                }
                ChangeMembers::RemoveNodes(remove_node_ids) => {
                    for node_id in &remove_node_ids {
                        target.nodes.remove(node_id);
                    }
                    None
                }
                ChangeMembers::ReplaceAllNodes(all_nodes) => {
                    target.nodes = all_nodes;
                    None
                }
                ChangeMembers::Batch(batch) => {
                    let changes = batch.into_iter().rev();
                    pending.extend(changes);
                    None
                }
            };

            if let Some(voter_ids) = new_voter_ids {
                target.configs = vec![voter_ids];
            }
        }

        let Some(target_voter_ids) = target.configs.pop() else {
            return Err(EmptyMembership {}.into());
        };
        target.configs = vec![target_voter_ids];
        tracing::debug!("target membership: {}", target);
        Ok(target)
    }

    fn add_learner(node_id: NID, node: N) -> Self {
        let nodes = BTreeMap::from([(node_id, node)]);
        ChangeMembers::AddNodes(nodes)
    }

    fn make_uniform() -> Self {
        ChangeMembers::AddVoterIds(BTreeSet::new())
    }
}

#[cfg(test)]
mod change_members_test;
