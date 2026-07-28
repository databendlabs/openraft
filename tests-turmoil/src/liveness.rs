//! Liveness-phase convergence check.
//!
//! After the safety phase, the fuzzer removes every fault (partitions, holds,
//! latency, loss, crashes) and requires the cluster to *completely heal*: one
//! leader, a settled (non-joint) membership, and every member holding an
//! identical, fully-applied log and state machine. A cluster that cannot reach
//! this state without faults has a liveness bug (stuck replication, dead-end
//! progress state, election live-lock, ...).

use std::collections::BTreeSet;

use crate::cluster::FullNodeSnapshot;
use crate::typ::NodeId;

/// The healed state: the leader and the members that were verified identical.
#[derive(Debug, Clone)]
pub struct Converged {
    pub leader: NodeId,
    /// All nodes of the final membership: voters and learners.
    pub members: BTreeSet<NodeId>,
}

/// Check whether the cluster has fully converged.
///
/// Returns `Err(reason)` describing the first unmet condition; the caller
/// retries until a deadline and reports the last reason on failure.
///
/// A node claiming leadership while absent from its own effective membership
/// is a stale ghost (e.g. an ex-leader removed from the config while
/// partitioned). It cannot serve requests and is expected to be deposed or
/// ignored; it does not count as "the" leader here. A leader that was demoted
/// to *learner* still counts: openraft keeps such a leader leading (it commits
/// through the voter quorum), and `StepDownWatcher` only steps down leaders
/// that were removed from the config entirely.
pub fn check_converged(snapshots: &[(NodeId, FullNodeSnapshot)]) -> Result<Converged, String> {
    let claimants: Vec<NodeId> =
        snapshots.iter().filter(|(_, s)| s.raft.state.is_leader()).map(|(id, _)| *id).collect();

    let leaders: Vec<NodeId> = claimants
        .iter()
        .copied()
        .filter(|id| {
            let (_, s) = snapshots.iter().find(|(sid, _)| sid == id).expect("claimant must be in snapshots");
            let m = s.raft.membership_config.membership();
            m.voter_ids().chain(m.learner_ids()).any(|member| member == *id)
        })
        .collect();

    if leaders.len() != 1 {
        return Err(format!(
            "expected exactly one leader, claimants={claimants:?}, self-member leaders={leaders:?}"
        ));
    }
    let leader = leaders[0];
    let (_, ls) = snapshots.iter().find(|(id, _)| *id == leader).expect("leader must be in snapshots");

    let membership = ls.raft.membership_config.membership();
    if membership.get_joint_config().len() != 1 {
        return Err(format!("leader n{leader} still in joint membership {membership}"));
    }

    let members: BTreeSet<NodeId> = membership.voter_ids().chain(membership.learner_ids()).collect();

    // The leader itself must have applied everything it holds.
    let leader_applied = ls.raft.last_applied;
    if leader_applied.map(|l| l.index) != ls.raft.last_log_index {
        return Err(format!(
            "leader n{leader} not fully applied: applied={:?}, last_log_index={:?}",
            leader_applied, ls.raft.last_log_index
        ));
    }

    for member in &members {
        let Some((_, s)) = snapshots.iter().find(|(id, _)| id == member) else {
            return Err(format!("member n{member} is not running"));
        };
        if s.raft.last_log_index != ls.raft.last_log_index {
            return Err(format!(
                "member n{member} last_log_index={:?} != leader's {:?}",
                s.raft.last_log_index, ls.raft.last_log_index
            ));
        }
        if s.raft.last_applied != leader_applied {
            return Err(format!(
                "member n{member} applied={:?} != leader's {:?}",
                s.raft.last_applied, leader_applied
            ));
        }
        if s.sm.last_applied != s.raft.last_applied {
            return Err(format!(
                "member n{member} state machine applied={:?} lags metrics applied={:?}",
                s.sm.last_applied, s.raft.last_applied
            ));
        }
        if s.sm.data != ls.sm.data {
            return Err(format!(
                "member n{member} state machine data differs from leader n{leader}"
            ));
        }
    }

    Ok(Converged { leader, members })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use openraft::ServerState;
    use openraft::metrics::RaftMetrics;
    use openraft::vote::RaftLeaderId;

    use super::*;
    use crate::store::StateMachineData;
    use crate::store::ValueMeta;
    use crate::typ::LogId;
    use crate::typ::StoredMembership;
    use crate::typ::TypeConfig;

    fn log_id(term: u64, node_id: NodeId, index: u64) -> LogId {
        LogId::new(openraft::impls::leader_id_adv::LeaderId::new(term, node_id), index)
    }

    /// A node fixture: `state`, voters/learners of its own effective config,
    /// log/applied position, and one data key to control SM equality.
    fn node(
        id: NodeId,
        state: ServerState,
        voters: Vec<Vec<NodeId>>,
        learners: Vec<NodeId>,
        applied: Option<LogId>,
        data_value: &str,
    ) -> (NodeId, FullNodeSnapshot) {
        let mut raft = RaftMetrics::<TypeConfig>::new_initial(id);
        raft.state = state;
        raft.last_log_index = applied.map(|l| l.index);
        raft.last_applied = applied;

        let sets: Vec<std::collections::BTreeSet<NodeId>> =
            voters.into_iter().map(|v| v.into_iter().collect()).collect();
        let mem = openraft::Membership::new_with_defaults(sets, learners);
        raft.membership_config = Arc::new(StoredMembership::new(Some(log_id(1, 1, 1)), mem));

        let mut sm = StateMachineData {
            last_applied: applied,
            ..Default::default()
        };
        sm.data.insert("k".to_string(), ValueMeta {
            value: data_value.to_string(),
            serial: 0,
            log_id: log_id(1, 1, 1),
        });

        (id, FullNodeSnapshot { raft, sm })
    }

    #[test]
    fn converged_cluster_accepted() {
        let applied = Some(log_id(2, 1, 10));
        let snapshots = vec![
            node(1, ServerState::Leader, vec![vec![1, 2, 3]], vec![4], applied, "v"),
            node(2, ServerState::Follower, vec![vec![1, 2, 3]], vec![4], applied, "v"),
            node(3, ServerState::Follower, vec![vec![1, 2, 3]], vec![4], applied, "v"),
            node(4, ServerState::Learner, vec![vec![1, 2, 3]], vec![4], applied, "v"),
        ];
        let converged = check_converged(&snapshots).unwrap();
        assert_eq!(converged.leader, 1);
        assert_eq!(converged.members, [1, 2, 3, 4].into_iter().collect());
    }

    #[test]
    fn no_leader_rejected() {
        let applied = Some(log_id(2, 1, 10));
        let snapshots = vec![
            node(1, ServerState::Follower, vec![vec![1, 2]], vec![], applied, "v"),
            node(2, ServerState::Follower, vec![vec![1, 2]], vec![], applied, "v"),
        ];
        assert!(check_converged(&snapshots).is_err());
    }

    #[test]
    fn ghost_leader_outside_its_config_ignored() {
        let applied = Some(log_id(2, 1, 10));
        // Node 9 claims leadership but is not a voter of its own config:
        // a stale ghost. Node 1 is the real leader.
        let snapshots = vec![
            node(1, ServerState::Leader, vec![vec![1, 2]], vec![], applied, "v"),
            node(2, ServerState::Follower, vec![vec![1, 2]], vec![], applied, "v"),
            node(9, ServerState::Leader, vec![vec![1, 2]], vec![], applied, "v"),
        ];
        let converged = check_converged(&snapshots).unwrap();
        assert_eq!(converged.leader, 1);
        assert!(!converged.members.contains(&9));
    }

    #[test]
    fn learner_leader_accepted() {
        let applied = Some(log_id(2, 1, 10));
        // Node 1 was demoted to learner but keeps leading: openraft only steps
        // down a leader that is removed from the config entirely.
        let snapshots = vec![
            node(1, ServerState::Leader, vec![vec![2, 3]], vec![1], applied, "v"),
            node(2, ServerState::Follower, vec![vec![2, 3]], vec![1], applied, "v"),
            node(3, ServerState::Follower, vec![vec![2, 3]], vec![1], applied, "v"),
        ];
        let converged = check_converged(&snapshots).unwrap();
        assert_eq!(converged.leader, 1);
        assert_eq!(converged.members, [1, 2, 3].into_iter().collect());
    }

    #[test]
    fn joint_membership_rejected() {
        let applied = Some(log_id(2, 1, 10));
        let snapshots = vec![
            node(
                1,
                ServerState::Leader,
                vec![vec![1, 2, 3], vec![1, 2]],
                vec![],
                applied,
                "v",
            ),
            node(
                2,
                ServerState::Follower,
                vec![vec![1, 2, 3], vec![1, 2]],
                vec![],
                applied,
                "v",
            ),
            node(
                3,
                ServerState::Follower,
                vec![vec![1, 2, 3], vec![1, 2]],
                vec![],
                applied,
                "v",
            ),
        ];
        let err = check_converged(&snapshots).unwrap_err();
        assert!(err.contains("joint"), "{err}");
    }

    #[test]
    fn lagging_member_rejected() {
        let applied = Some(log_id(2, 1, 10));
        let behind = Some(log_id(2, 1, 7));
        let snapshots = vec![
            node(1, ServerState::Leader, vec![vec![1, 2]], vec![], applied, "v"),
            node(2, ServerState::Follower, vec![vec![1, 2]], vec![], behind, "v"),
        ];
        let err = check_converged(&snapshots).unwrap_err();
        assert!(err.contains("n2"), "{err}");
    }

    #[test]
    fn missing_member_rejected() {
        let applied = Some(log_id(2, 1, 10));
        let snapshots = vec![node(1, ServerState::Leader, vec![vec![1, 2]], vec![], applied, "v")];
        let err = check_converged(&snapshots).unwrap_err();
        assert!(err.contains("n2"), "{err}");
    }

    #[test]
    fn diverged_state_machine_rejected() {
        let applied = Some(log_id(2, 1, 10));
        let snapshots = vec![
            node(1, ServerState::Leader, vec![vec![1, 2]], vec![], applied, "v1"),
            node(2, ServerState::Follower, vec![vec![1, 2]], vec![], applied, "v2"),
        ];
        let err = check_converged(&snapshots).unwrap_err();
        assert!(err.contains("data differs"), "{err}");
    }
}
