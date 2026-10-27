use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::btree_set;
use std::fmt;

use openraft::errors::MembershipError;
use openraft::impls::ChangeMembers;
use openraft::membership::NodeRef;
use openraft::membership::NodeRole;
use openraft::membership::QuorumBridge;
use openraft::membership::QuorumIntersection;
use openraft::membership::QuorumSet;
use openraft::membership::RaftMembership;
use openraft::membership::RaftMembershipEdit;
use serde::Deserialize;
use serde::Serialize;

/// A quorum rule that accepts a set of voters when the set contains one of the listed quorums.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MinimalQuorums(pub Vec<BTreeSet<u64>>);

impl QuorumSet for MinimalQuorums {
    type Id = u64;
    type Iter = btree_set::IntoIter<u64>;

    fn is_quorum<'a, I>(&self, ids: I) -> bool
    where I: Iterator<Item = &'a u64> + Clone {
        let ids: BTreeSet<u64> = ids.copied().collect();
        self.0.iter().any(|quorum| quorum.is_subset(&ids))
    }

    fn ids(&self) -> Self::Iter {
        let mut ids = BTreeSet::new();
        for quorum in &self.0 {
            ids.extend(quorum);
        }
        ids.into_iter()
    }
}

impl QuorumIntersection<MinimalQuorums> for MinimalQuorums {
    /// Every quorum contains a listed quorum, so two rules intersect exactly when each listed
    /// quorum of one meets each listed quorum of the other.
    fn intersects_with(&self, other: &MinimalQuorums) -> Option<bool> {
        for quorum in &self.0 {
            for other_quorum in &other.0 {
                if quorum.is_disjoint(other_quorum) {
                    return Some(false);
                }
            }
        }
        Some(true)
    }
}

impl QuorumBridge<MinimalQuorums> for MinimalQuorums {
    fn joint_with(&self, _other: MinimalQuorums) -> Self {
        unimplemented!("the test never changes the membership")
    }
}

/// A membership whose read and write quorum rules differ, with configurable node roles.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MinimalQuorumsMembership {
    pub read: MinimalQuorums,
    pub write: MinimalQuorums,
    /// Explicit roles and extra known nodes. Voters default to `Campaigner | ReceiveLogs`.
    pub roles: BTreeMap<u64, NodeRole>,
}

impl fmt::Display for MinimalQuorumsMembership {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{{read: {:?}, write: {:?}, roles: {:?}}}",
            self.read.0, self.write.0, self.roles
        )
    }
}

impl RaftMembership for MinimalQuorumsMembership {
    type NodeId = u64;
    type Node = ();

    type QuorumSet = MinimalQuorums;

    fn read_quorum_set(&self) -> &MinimalQuorums {
        &self.read
    }

    fn write_quorum_set(&self) -> &MinimalQuorums {
        &self.write
    }

    fn supports_direct_transition_to(&self, other: &Self) -> bool {
        let read_meets_write = self.read.intersects_with(&other.write);
        if read_meets_write != Some(true) {
            return false;
        }

        let write_meets_read = self.write.intersects_with(&other.read);
        write_meets_read == Some(true)
    }

    fn get_node(&self, node_id: &u64) -> Option<NodeRef<'_, ()>> {
        let is_read_voter = self.read.ids().any(|id| id == *node_id);
        let is_write_voter = self.write.ids().any(|id| id == *node_id);
        let role = if let Some(role) = self.roles.get(node_id) {
            *role
        } else if is_read_voter || is_write_voter {
            NodeRole::Campaigner | NodeRole::ReceiveLogs
        } else {
            return None;
        };

        let node_ref = NodeRef {
            node: &(),
            role,
            is_read_voter,
            is_write_voter,
        };
        Some(node_ref)
    }

    type Iter = btree_set::IntoIter<u64>;

    fn node_ids(&self) -> Self::Iter {
        let mut ids: BTreeSet<u64> = self.read.ids().collect();
        ids.extend(self.write.ids());
        ids.extend(self.roles.keys());
        ids.into_iter()
    }

    fn new<I>(_read_quorum_set: MinimalQuorums, _write_quorum_set: MinimalQuorums, _nodes: I) -> Self
    where I: IntoIterator<Item = (u64, (), NodeRole)> {
        unimplemented!("the test never changes the membership")
    }
}

impl RaftMembershipEdit<MinimalQuorumsMembership> for ChangeMembers<u64, ()> {
    type Error = std::convert::Infallible;

    fn apply(
        self,
        _membership: &MinimalQuorumsMembership,
    ) -> Result<MinimalQuorumsMembership, MembershipError<u64, Self::Error>> {
        unimplemented!("the test never changes the membership")
    }

    fn add_learner(node_id: u64, node: ()) -> Self {
        let nodes = BTreeMap::from([(node_id, node)]);
        ChangeMembers::AddNodes(nodes)
    }

    fn make_uniform() -> Self {
        ChangeMembers::AddVoterIds(BTreeSet::new())
    }
}
