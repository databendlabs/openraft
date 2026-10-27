use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;

use maplit::btreemap;
use maplit::btreeset;

use crate::membership::NodeRef;
use crate::membership::NodeRole;
use crate::membership::RaftMembership;
use crate::membership::RaftMembershipExt;
use crate::membership::errors::UnprovenReadWriteIntersection;
use crate::membership::errors::WriteVoterWithoutReceiveLogs;
use crate::quorum::QuorumIntersection;

/// A configuration that stores the role of each node, so that a node outside the quorum sets
/// can campaign. Its read and write quorum sets may differ.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
struct RoleMembership {
    read: Vec<BTreeSet<u64>>,
    write: Vec<BTreeSet<u64>>,
    roles: BTreeMap<u64, NodeRole>,
}

impl fmt::Display for RoleMembership {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl RaftMembership for RoleMembership {
    type NodeId = u64;
    type Node = ();

    type QuorumSet = Vec<BTreeSet<u64>>;

    fn read_quorum_set(&self) -> &Self::QuorumSet {
        &self.read
    }

    fn write_quorum_set(&self) -> &Self::QuorumSet {
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
        let role = *self.roles.get(node_id)?;
        let is_read_voter = self.read.iter().any(|ids| ids.contains(node_id));
        let is_write_voter = self.write.iter().any(|ids| ids.contains(node_id));
        let node_ref = NodeRef {
            node: &(),
            role,
            is_read_voter,
            is_write_voter,
        };
        Some(node_ref)
    }

    type Iter = std::vec::IntoIter<u64>;

    fn node_ids(&self) -> Self::Iter {
        let keys = self.roles.keys();
        let ids: Vec<u64> = keys.copied().collect();
        ids.into_iter()
    }

    fn new<I>(read_quorum_set: Self::QuorumSet, write_quorum_set: Self::QuorumSet, nodes: I) -> Self
    where I: IntoIterator<Item = (u64, (), NodeRole)> {
        let mut roles = BTreeMap::new();
        for (node_id, _node, role) in nodes {
            roles.insert(node_id, role);
        }

        RoleMembership {
            read: read_quorum_set,
            write: write_quorum_set,
            roles,
        }
    }
}

/// A [`RoleMembership`] whose type vouches that its read quorums intersect its write quorums.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
struct TrustedMembership(RoleMembership);

impl fmt::Display for TrustedMembership {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl RaftMembership for TrustedMembership {
    type NodeId = u64;
    type Node = ();

    type QuorumSet = Vec<BTreeSet<u64>>;

    fn read_quorum_set(&self) -> &Self::QuorumSet {
        self.0.read_quorum_set()
    }

    fn write_quorum_set(&self) -> &Self::QuorumSet {
        self.0.write_quorum_set()
    }

    fn read_write_quorums_intersect(&self) -> Option<bool> {
        Some(true)
    }

    fn supports_direct_transition_to(&self, other: &Self) -> bool {
        self.0.supports_direct_transition_to(&other.0)
    }

    fn get_node(&self, node_id: &u64) -> Option<NodeRef<'_, ()>> {
        self.0.get_node(node_id)
    }

    type Iter = std::vec::IntoIter<u64>;

    fn node_ids(&self) -> Self::Iter {
        self.0.node_ids()
    }

    fn new<I>(read_quorum_set: Self::QuorumSet, write_quorum_set: Self::QuorumSet, nodes: I) -> Self
    where I: IntoIterator<Item = (u64, (), NodeRole)> {
        let membership = RoleMembership::new(read_quorum_set, write_quorum_set, nodes);
        Self(membership)
    }
}

#[test]
fn drop_voters_demoted_to_learners_keeps_campaigners() {
    let voter = NodeRole::Campaigner | NodeRole::ReceiveLogs;
    let learner = NodeRole::ReceiveLogs;
    let current = RoleMembership {
        read: vec![btreeset! {1, 2, 3}],
        write: vec![btreeset! {1, 2, 3}],
        roles: btreemap! {1 => voter, 2 => voter, 3 => voter, 4 => learner},
    };
    // Voters 2 and 3 leave the quorum sets: 2 keeps campaigning, and 3 becomes a learner.
    let goal = RoleMembership {
        read: vec![btreeset! {1}],
        write: vec![btreeset! {1}],
        roles: btreemap! {1 => voter, 2 => voter, 3 => learner, 4 => learner},
    };

    let res = current.drop_voters_demoted_to_learners(goal);

    let want = RoleMembership {
        read: vec![btreeset! {1}],
        write: vec![btreeset! {1}],
        roles: btreemap! {1 => voter, 2 => voter, 4 => learner},
    };
    assert_eq!(want, res);
}

#[test]
fn ensure_read_write_quorums_intersect_accepts_unproven_only_when_overridden() {
    let with_quorum_sets = |read, write| RoleMembership {
        read,
        write,
        roles: BTreeMap::new(),
    };

    // Every majority of `{1,2}` intersects every majority of `{1,2,3}`.
    let proven = with_quorum_sets(vec![btreeset! {1, 2}], vec![btreeset! {1, 2, 3}]);

    let res = proven.ensure_read_write_quorums_intersect();
    assert_eq!(Ok(()), res);

    // Read quorum `{1,2}` misses write quorum `{4,5}`.
    let disjoint = with_quorum_sets(vec![btreeset! {1, 2, 3}], vec![btreeset! {3, 4, 5}]);
    let want = Err(UnprovenReadWriteIntersection {
        membership: disjoint.to_string(),
    });

    let res = disjoint.ensure_read_write_quorums_intersect();
    assert_eq!(want, res);

    // Every read quorum holds at least two of `{2,3,4,5}`, and every write quorum holds three
    // of them, so they intersect. No pair of configs shows it: majority `{1,2}` of `{1,2,3}`
    // and majority `{1,4}` of `{1,4,5}` each miss a majority of `{2,3,4,5}`, so
    // `intersects_with()` returns `None`.
    let read = vec![btreeset! {1, 2, 3}, btreeset! {1, 4, 5}];
    let write = vec![btreeset! {2, 3, 4, 5}];
    let unproven = with_quorum_sets(read, write);
    let want = Err(UnprovenReadWriteIntersection {
        membership: unproven.to_string(),
    });

    let res = unproven.ensure_read_write_quorums_intersect();
    assert_eq!(want, res);

    // The type overrides `read_write_quorums_intersect()` to vouch for the same quorum sets.
    let trusted = TrustedMembership(unproven);

    let res = trusted.ensure_read_write_quorums_intersect();
    assert_eq!(Ok(()), res);
}

#[test]
fn ensure_write_voters_receive_logs_checks_only_write_voters() {
    let voter = NodeRole::Campaigner | NodeRole::ReceiveLogs;

    // Read voter 3 only votes, so it needs no logs.
    let valid = RoleMembership {
        read: vec![btreeset! {1, 2, 3}],
        write: vec![btreeset! {1, 2}],
        roles: btreemap! {1 => voter, 2 => voter, 3 => NodeRole::Campaigner},
    };

    let res = valid.ensure_write_voters_receive_logs();
    assert_eq!(Ok(()), res);

    // Write voter 2 receives no logs, so it never acknowledges one.
    let invalid = RoleMembership {
        read: vec![btreeset! {1, 2, 3}],
        write: vec![btreeset! {1, 2}],
        roles: btreemap! {1 => voter, 2 => NodeRole::Campaigner, 3 => voter},
    };
    let want = Err(WriteVoterWithoutReceiveLogs { node_id: 2 });

    let res = invalid.ensure_write_voters_receive_logs();
    assert_eq!(want, res);
}

#[test]
fn joint_with_joins_read_and_write_quorum_sets_separately() {
    let voter = NodeRole::Campaigner | NodeRole::ReceiveLogs;
    let learner = NodeRole::ReceiveLogs;
    let current = RoleMembership {
        read: vec![btreeset! {1, 2}],
        write: vec![btreeset! {1, 2, 3}],
        roles: btreemap! {1 => voter, 2 => voter, 3 => voter, 4 => learner, 5 => learner},
    };
    let goal = RoleMembership {
        read: vec![btreeset! {4, 5}],
        write: vec![btreeset! {3, 4, 5}],
        roles: btreemap! {1 => learner, 2 => learner, 3 => voter, 4 => voter, 5 => voter},
    };

    let joint = current.joint_with(&goal);

    // The read quorum set of the joint holds the read configs of both sides, and its write
    // quorum set holds their write configs. Voters 1 and 2 keep their roles in `current`.
    let want = RoleMembership {
        read: vec![btreeset! {1, 2}, btreeset! {4, 5}],
        write: vec![btreeset! {1, 2, 3}, btreeset! {3, 4, 5}],
        roles: btreemap! {1 => voter, 2 => voter, 3 => voter, 4 => voter, 5 => voter},
    };
    assert_eq!(want, joint);
}

#[test]
fn joint_with_joins_old_and_goal_quorum_sets() {
    let voter = NodeRole::Campaigner | NodeRole::ReceiveLogs;
    let learner = NodeRole::ReceiveLogs;
    let current = RoleMembership {
        read: vec![btreeset! {1, 2}],
        write: vec![btreeset! {1, 2, 3}],
        roles: btreemap! {1 => voter, 2 => voter, 3 => voter, 4 => learner},
    };
    let goal = RoleMembership {
        read: vec![btreeset! {1, 2, 4}],
        write: vec![btreeset! {1, 2, 4}],
        roles: btreemap! {1 => voter, 2 => voter, 3 => learner, 4 => voter},
    };

    let joint = current.joint_with(&goal);

    // Every read quorum of the joint is a quorum of `{1,2}`, so it holds 1 and 2 and meets every
    // write quorum of `current`. Voter 3 keeps its roles in `current`.
    let want = RoleMembership {
        read: vec![btreeset! {1, 2}, btreeset! {1, 2, 4}],
        write: vec![btreeset! {1, 2, 3}, btreeset! {1, 2, 4}],
        roles: btreemap! {1 => voter, 2 => voter, 3 => voter, 4 => voter},
    };
    assert_eq!(want, joint);
}
