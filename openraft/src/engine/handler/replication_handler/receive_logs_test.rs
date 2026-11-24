//! Tests of a target that stops and starts receiving logs. The default membership gives every node
//! `NodeRole::ReceiveLogs`, so these tests use a membership that stores the roles of its
//! non-voters.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use maplit::btreemap;
use maplit::btreeset;
use openraft_rt_tokio::TokioRuntime;
use pretty_assertions::assert_eq;

use crate::MembershipState;
use crate::Vote;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::engine::TargetProgress;
use crate::engine::testing::log_id;
use crate::errors::MembershipError;
use crate::membership::NodeRef;
use crate::membership::NodeRole;
use crate::membership::RaftMembership;
use crate::membership::RaftMembershipEdit;
use crate::progress::Inflight;
use crate::progress::entry::ProgressEntry;
use crate::progress::inflight_id::InflightId;
use crate::raft_state::LogStateReader;
use crate::replication::payload::Payload;
use crate::replication::replicate::Replicate;
use crate::replication::response::Progress;
use crate::replication::response::ReplicationResult;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::StoredMembershipOf;
use crate::utime::Leased;
use crate::vote::raft_vote::RaftVoteExt;

/// A membership with one voter set, which is both the read and the write quorum. A voter has
/// every role; a non-voter has the role that `roles` gives it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
struct RoleMembership {
    voters: BTreeSet<u64>,
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

    fn read_quorum_set(&self) -> Self::QuorumSet {
        vec![self.voters.clone()]
    }

    fn write_quorum_set(&self) -> Self::QuorumSet {
        vec![self.voters.clone()]
    }

    /// The tests change only the roles of non-voters, so every transition keeps the quorums.
    fn supports_direct_transition_to(&self, other: &Self) -> bool {
        self.voters == other.voters
    }

    fn get_node(&self, node_id: &u64) -> Option<NodeRef<'_, ()>> {
        let is_voter = self.voters.contains(node_id);
        let role = match self.roles.get(node_id) {
            Some(role) => *role,
            None if is_voter => NodeRole::Campaigner | NodeRole::ReceiveLogs,
            None => return None,
        };
        let node_ref = NodeRef {
            node: &(),
            role,
            is_read_voter: is_voter,
            is_write_voter: is_voter,
        };
        Some(node_ref)
    }

    type Iter = std::collections::btree_set::IntoIter<u64>;

    fn node_ids(&self) -> Self::Iter {
        let ids: BTreeSet<u64> = self.voters.iter().chain(self.roles.keys()).copied().collect();
        ids.into_iter()
    }

    fn joint_with(&self, _goal: &Self) -> Self {
        unimplemented!("the tests keep the quorums, so they need no joint config")
    }
}

/// A membership edit that the tests never apply: they append memberships to the engine directly.
#[derive(Debug)]
struct NoEdit;

impl fmt::Display for NoEdit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NoEdit")
    }
}

impl RaftMembershipEdit<RoleMembership> for NoEdit {
    type Error = fmt::Error;

    fn apply(
        self,
        _membership: &RoleMembership,
        _retain: bool,
    ) -> Result<RoleMembership, MembershipError<u64, fmt::Error>> {
        unimplemented!("the tests append memberships directly")
    }
}

crate::declare_raft_types!(
    RoleConfig:
        D = u64,
        R = (),
        Node = (),
        Membership = RoleMembership,
        MembershipEdit = NoEdit,
        ErrorSource = anyerror::AnyError,
        AsyncRuntime = TokioRuntime,
);

/// Voter 1, and the non-voters 2 and 3, of which 3 always receives logs.
fn roles(role_of_2: NodeRole) -> RoleMembership {
    RoleMembership {
        voters: btreeset! {1},
        roles: btreemap! {2 => role_of_2, 3 => NodeRole::ReceiveLogs},
    }
}

/// Node 1 leads in term 3, with logs up to (2, 1, 3), and nodes 2 and 3 have matched log 1.
/// Node 3 streams the logs after it under inflight id 6, and node 2 runs `inflight_of_2`.
fn eng(inflight_of_2: Inflight<RoleConfig>) -> Engine<RoleConfig> {
    let mut eng = Engine::testing_default(1);
    eng.state.vote = Leased::new(RoleConfig::now(), Duration::from_millis(500), Vote::new_committed(3, 1));
    let stored = Arc::new(StoredMembershipOf::<RoleConfig>::new(
        Some(log_id(1, 1, 1)),
        roles(NodeRole::ReceiveLogs),
    ));
    eng.state.membership_state = MembershipState::new(stored.clone(), stored);
    eng.state.log_ids = LogIdList::new(None, [log_id(1, 1, 1), log_id(2, 1, 3)]);
    eng.state.server_state = eng.calc_server_state();
    eng.state.last_inflight_id = 10;
    eng.testing_new_leader();
    eng.output.clear_commands();

    let logs_since_1 = Inflight::logs_since(Some(log_id(1, 1, 1)), InflightId::new(6));
    for (target, inflight) in [(2, inflight_of_2), (3, logs_since_1)] {
        eng.leader.as_mut().unwrap().progress.update_entry_with(&target, |entry| {
            entry.matching = Some(log_id(1, 1, 1));
            entry.data.searching_end = 2;
            entry.data.inflight = inflight;
        });
    }
    eng
}

fn entry_of(eng: &Engine<RoleConfig>, target: u64) -> ProgressEntry<RoleConfig> {
    eng.leader.as_ref().unwrap().progress.try_get(&target).unwrap().clone()
}

/// The `RebuildReplicationStreams` that rebuilds the streams of nodes 2 and 3 for `progresses`.
fn rebuild(progresses: [ProgressEntry<RoleConfig>; 2]) -> Command<RoleConfig> {
    let targets = progresses
        .into_iter()
        .map(|progress| TargetProgress {
            target_node: (),
            progress,
        })
        .collect();
    Command::RebuildReplicationStreams {
        leader_vote: Vote::new_committed(3, 1).to_committed(),
        targets,
        close_old_streams: false,
    }
}

/// When node 2 stops receiving logs, RaftCore closes its stream, so the leader drops the work the
/// stream had in flight, whatever it is; node 2 keeps its matching log id, and node 3 its work.
/// When node 2 receives logs again, it gets a new stream and new work, which an answer to the
/// dropped work does not change.
#[test]
fn test_stop_and_restart_receiving_logs() {
    let current = InflightId::new(5);
    let probe = Inflight::probe(Some(log_id(1, 1, 1)), Some(log_id(2, 1, 3)), current);
    let logs_since = Inflight::logs_since(Some(log_id(1, 1, 1)), current);
    let clear = Inflight::Clear {
        prev: Some(log_id(1, 1, 1)),
        inflight_id: current,
    };

    for inflight in [logs_since, probe, Inflight::snapshot(current), clear] {
        let mut eng = eng(inflight);
        let entry_of_3 = entry_of(&eng, 3);

        tracing::info!(%inflight, "--- node 2 stops receiving logs: the leader drops its work in flight");
        {
            let mut dropped = entry_of(&eng, 2);
            dropped.data.inflight = Inflight::None;

            eng.replication_handler().append_membership(&log_id(3, 1, 4), &roles(NodeRole::Empty));
            eng.replication_handler().initiate_replication();

            assert_eq!(dropped, entry_of(&eng, 2));
            assert_eq!(entry_of_3, entry_of(&eng, 3));
            let expected = vec![rebuild([dropped, entry_of_3.clone()])];
            assert_eq!(expected, eng.output.take_commands());
        }

        tracing::info!(%inflight, "--- node 2 receives logs again: it gets new work, after its matching log");
        let entry_of_2 = {
            eng.replication_handler().append_membership(&log_id(3, 1, 5), &roles(NodeRole::ReceiveLogs));
            let idle = entry_of(&eng, 2);
            assert_eq!(Inflight::None, idle.data.inflight);
            eng.replication_handler().initiate_replication();

            let expected = vec![rebuild([idle, entry_of_3.clone()]), Command::Replicate {
                target: 2,
                req: Replicate {
                    inflight_id: InflightId::new(11),
                    payload: Some(Payload::LogsSince {
                        prev: Some(log_id(1, 1, 1)),
                    }),
                },
            }];
            assert_eq!(expected, eng.output.take_commands());
            entry_of(&eng, 2)
        };

        tracing::info!(%inflight, "--- an answer to the dropped work does not change the new work");
        {
            let stream_id = entry_of_2.data.stream_id;
            let ack = Progress {
                target: 2,
                result: Ok(ReplicationResult(Ok(Some(log_id(2, 1, 3))))),
                hint: None,
            };
            let error = Progress {
                target: 2,
                result: Err("connection reset".to_string()),
                hint: None,
            };
            eng.replication_handler().update_progress(stream_id, ack, Some(current));
            eng.replication_handler().update_progress(stream_id, error, Some(current));

            assert_eq!(entry_of_2, entry_of(&eng, 2));
            assert_eq!(Vec::<Command<RoleConfig>>::new(), eng.output.take_commands());
        }
    }
}

/// When node 2 stops receiving logs, the work it drops no longer holds back a pending purge, which
/// then runs right after the streams are rebuilt.
#[test]
fn test_stop_receiving_logs_releases_a_pending_purge() {
    let mut eng = eng(Inflight::logs_since(Some(log_id(1, 1, 1)), InflightId::new(5)));
    eng.state.enable_validation(false); // The test sets a purge that waits.
    eng.leader.as_mut().unwrap().progress.update_data_with(&3, |data| data.inflight = Inflight::None);
    eng.state.purge_upto = Some(log_id(2, 1, 3));

    tracing::info!("--- only node 2 reads the logs that wait to be purged");
    {
        eng.replication_handler().try_purge_log();
        assert_eq!(Vec::<Command<RoleConfig>>::new(), eng.output.take_commands());
    }

    tracing::info!("--- node 2 stops receiving logs: its work is dropped, and the purge runs");
    {
        let mut dropped = entry_of(&eng, 2);
        dropped.data.inflight = Inflight::None;
        let entry_of_3 = entry_of(&eng, 3);

        eng.replication_handler().append_membership(&log_id(3, 1, 4), &roles(NodeRole::Empty));

        let expected = vec![rebuild([dropped, entry_of_3]), Command::PurgeLog {
            upto: log_id(2, 1, 3),
        }];
        assert_eq!(expected, eng.output.take_commands());
        assert_eq!(Some(&log_id(2, 1, 3)), eng.state.last_purged_log_id());
    }
}
