use std::collections::BTreeSet;

use openraft_macros::since;

use crate::node::NodeId;

/// The membership change would enter a joint membership that leaves out the leader proposing it.
///
/// [`Raft::change_membership()`] writes a joint membership and then, once that is committed, the
/// uniform membership that completes it, both from the same leader. A leader changes the
/// membership only while it is in it, so a leader that is neither a voter nor a learner of the
/// joint membership could not write the second entry. The change is refused before anything is
/// written, instead of leaving the cluster in a joint membership this leader cannot complete.
///
/// It happens when the membership is already joint, the leader is in its first voter set only, and
/// the requested voters leave it out as well. The change bridges through the last voter set:
///
/// ```text
/// [{a,b,c}, {b,c,d}], leader a, voters {b,c} -> [{b,c,d}, {b,c}], without a
/// ```
///
/// Complete the current joint membership first, by changing to one of its voter sets, here
/// `{a,b,c}` or `{b,c,d}`, then ask the leader of the result for the voters wanted. Or pass
/// `retain = true`: the leader stays in the joint membership as a learner, and completes the
/// change.
///
/// [`Raft::change_membership()`]: crate::raft::Raft::change_membership
#[since(version = "0.10.0")]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize), serde(bound = ""))]
#[error(
    "leader {leader_id} would not be in the joint membership {joint:?}, and could not complete it: complete the current joint membership first"
)]
pub struct LeaderNotInJointMembership<NID>
where NID: NodeId
{
    /// The leader asked for the change.
    pub leader_id: NID,

    /// The voter sets of the joint membership the change would enter.
    pub joint: Vec<BTreeSet<NID>>,
}
