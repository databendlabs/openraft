use openraft_macros::since;

use crate::membership::RaftMembership;
use crate::membership::RaftMembershipEdit;

/// Optional capability for constructing a learner-addition edit.
///
/// Implement this alongside [`RaftMembershipEdit`] to enable
/// [`Raft::add_learner()`](crate::Raft::add_learner). Edits without this capability can still use
/// the general membership APIs. This trait constructs an edit; OpenRaft submits it and handles
/// the optional replication catch-up wait.
#[since(version = "0.10.0")]
pub trait RaftAddLearner<M>: RaftMembershipEdit<M>
where M: RaftMembership
{
    /// Returns an edit that adds `node_id` with `node` as a learner.
    ///
    /// Applying this edit must preserve quorum rules, existing node roles, and existing node
    /// metadata. Adding an existing voter or learner leaves its role and node unchanged. A new
    /// node receives logs but has no campaign, read voter, or write voter role.
    ///
    /// # Examples
    ///
    /// ```
    /// use openraft::impls::{ChangeMembers, Membership};
    /// use openraft::membership::{RaftAddLearner, RaftMembershipEdit};
    ///
    /// type Members = Membership<u64, String>;
    /// let membership = Members::new(
    ///     vec![std::collections::BTreeSet::from([1])],
    ///     std::collections::BTreeMap::from([(1, "original".to_string())]),
    /// ).unwrap();
    /// let add = <ChangeMembers<u64, String> as RaftAddLearner<Members>>::add_learner;
    /// let target = add(2, "learner".to_string()).apply(&membership, true).unwrap();
    /// assert_eq!(Some(&"learner".to_string()), target.get_node(&2));
    /// assert_eq!(membership, add(1, "replacement".to_string()).apply(&membership, true).unwrap());
    /// ```
    #[since(version = "0.10.0")]
    fn add_learner(node_id: M::NodeId, node: M::Node) -> Self;
}
