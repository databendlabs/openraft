use std::time::Duration;

use crate::RaftState;
use crate::RaftTypeConfig;
use crate::engine::Command;
use crate::engine::EngineConfig;
use crate::engine::EngineOutput;
use crate::engine::handler::replication_handler::ReplicationHandler;
use crate::engine::leader_log_ids::LeaderLogIds;
use crate::entry::RaftEntry;
use crate::proposer::Leader;
use crate::proposer::LeaderQuorumSet;
use crate::raft::linearizable_read::ReadLogId;
use crate::raft::message::TransferLeaderRequest;
use crate::raft_state::IOId;
use crate::replication::ReplicationSessionId;
use crate::rt::Instant;
use crate::storage::RaftStateMachine;
use crate::type_config::TypeConfigExt;
use crate::type_config::alias::BatchOf;
use crate::type_config::alias::CommittedLeaderIdOf;
use crate::type_config::alias::InstantOf;
use crate::type_config::alias::PayloadOf;

#[cfg(test)]
mod append_entries_test;
#[cfg(test)]
mod get_read_log_id_test;
#[cfg(test)]
mod send_heartbeat_test;
#[cfg(test)]
mod transfer_leader_test;

/// Handle leader operations.
///
/// - Append new logs;
/// - Change membership;
/// - etc.
pub(crate) struct LeaderHandler<'x, C, SM = ()>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    pub(crate) config: &'x mut EngineConfig<C>,
    pub(crate) leader: &'x mut Leader<C, LeaderQuorumSet<C>>,
    pub(crate) state: &'x mut RaftState<C>,
    pub(crate) output: &'x mut EngineOutput<C, SM>,
}

impl<'x, C, SM> LeaderHandler<'x, C, SM>
where
    C: RaftTypeConfig,
    SM: RaftStateMachine<C>,
{
    pub(crate) fn new(
        config: &'x mut EngineConfig<C>,
        leader: &'x mut Leader<C, LeaderQuorumSet<C>>,
        state: &'x mut RaftState<C>,
        output: &'x mut EngineOutput<C, SM>,
    ) -> Self {
        Self {
            config,
            leader,
            state,
            output,
        }
    }

    /// Return whether the leader's lease is valid.
    pub(crate) fn is_lease_valid(&self) -> bool {
        self.leader.is_lease_valid(self.config.timer_config.leader_lease)
    }

    /// Append new log entries by a leader.
    ///
    /// Also Update effective membership if the payload contains
    /// membership config.
    ///
    /// If there is a membership config log entry, the caller has to guarantee the previous one is
    /// committed.
    ///
    /// TODO(xp): if vote indicates this node is not the leader, refuse append
    #[tracing::instrument(level = "debug", skip(self, payloads))]
    pub(crate) fn leader_append_entries<I>(&mut self, payloads: I) -> Option<LeaderLogIds<CommittedLeaderIdOf<C>>>
    where I: IntoIterator<Item = PayloadOf<C>> + AsRef<[PayloadOf<C>]> {
        let log_ids = self.leader.assign_log_ids(payloads.as_ref().len())?;

        self.state.extend_log_ids_from_same_leader(log_ids.clone());

        let mut membership_entry = None;
        let entries: BatchOf<C, _> = payloads
            .into_iter()
            .zip(log_ids.clone())
            .map(|(payload, log_id)| {
                tracing::debug!("assign log id: {}", log_id);
                let entry = C::Entry::new(log_id, payload);
                if let Some(m) = entry.get_membership() {
                    debug_assert!(
                        membership_entry.is_none(),
                        "only one membership entry is allowed in a batch"
                    );
                    membership_entry = Some((entry.log_id(), m));
                }
                entry
            })
            .collect();

        self.state.accept_log_io(IOId::new_log_io(
            self.leader.committed_vote.clone(),
            self.leader.last_log_id().cloned(),
        ));

        self.output.push_command(Command::AppendEntries {
            // A leader should always use the leader's vote.
            // It is allowed to be different from local vote.
            committed_vote: self.leader.committed_vote.clone(),
            entries,
        });

        let mut rh = self.replication_handler();

        // Since this entry, the condition to commit has been changed.
        // But we only need to commit in the new membership config.
        // Because any quorum in the new one intersects with one in the previous membership config.
        if let Some((log_id, m)) = membership_entry {
            rh.append_membership(&log_id, &m);
        }

        rh.initiate_replication();

        Some(log_ids)
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn send_heartbeat(&mut self, bypass_min_interval: bool) -> bool {
        if !self.heartbeat_is_allowed(C::now()) {
            return false;
        }

        let membership_log_id = self.state.membership_state.effective().log_id();
        let session_id = ReplicationSessionId::new(self.leader.committed_vote.clone(), membership_log_id.clone());

        self.output.push_command(Command::BroadcastHeartbeat {
            session_id,
            bypass_min_interval,
        });
        true
    }

    fn heartbeat_is_allowed(&self, now: InstantOf<C>) -> bool {
        let Some(period) = self.config.quorum_loss_probe_interval else {
            return true;
        };
        let Some(age) = self.quorum_activity_age(now) else {
            return true;
        };
        let leader_lease = self.config.timer_config.leader_lease;
        if age < leader_lease {
            return true;
        }

        debug_assert!(!period.is_zero());
        let elapsed = age - leader_lease;
        let slot = elapsed.as_nanos() / period.as_nanos();
        slot % 2 == 1
    }

    fn quorum_activity_age(&self, now: InstantOf<C>) -> Option<Duration> {
        if self.leader.is_self_quorum() {
            return None;
        }

        let quorum_acked_at = self.leader.last_quorum_acked_time();
        let activity_at = quorum_acked_at.or(self.state.vote_last_modified())?;
        Some(now.saturating_duration_since(activity_at))
    }

    /// Get the log id for a linearizable read.
    ///
    /// See: [Read Operation](crate::docs::protocol::read)
    pub(crate) fn get_read_log_id(&self) -> ReadLogId<C> {
        ReadLogId::new(self.leader.noop_log_id.clone(), self.state.local_committed().cloned())
    }

    /// Disable proposing new logs for this Leader and transfer Leader to another node
    pub(crate) fn transfer_leader(&mut self, to: C::NodeId) {
        self.leader.mark_transfer(to.clone());
        self.state.vote.disable_lease();

        self.output.push_command(Command::BroadcastTransferLeader {
            req: TransferLeaderRequest::new(
                self.leader.committed_vote.clone().into_vote(),
                to,
                self.leader.last_log_id().cloned(),
            ),
        });
    }

    /// Return a replication session id
    #[allow(dead_code)]
    pub(crate) fn replication_session_id(&self) -> ReplicationSessionId<C> {
        let committed_vote = self.leader.committed_vote.clone();
        let membership_log_id = self.state.membership_state.effective().log_id();
        ReplicationSessionId::new(committed_vote, membership_log_id.clone())
    }

    pub(crate) fn replication_handler(&mut self) -> ReplicationHandler<'_, C, SM> {
        ReplicationHandler::new(self.config, self.leader, self.state, self.output)
    }
}
