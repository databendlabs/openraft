use crate::RaftState;
use crate::RaftTypeConfig;
use crate::base::batch::Batch;
use crate::engine::Command;
use crate::engine::EngineConfig;
use crate::engine::EngineOutput;
use crate::engine::handler::replication_handler::ReplicationHandler;
use crate::entry::RaftEntry;
use crate::entry::RaftPayload;
use crate::entry::payload::EntryPayload;
use crate::proposer::Leader;
use crate::proposer::LeaderQuorumSet;
use crate::raft::message::TransferLeaderRequest;
use crate::raft_state::IOId;
use crate::replication::ReplicationSessionId;
use crate::type_config::alias::LogIdOf;

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
pub(crate) struct LeaderHandler<'x, C>
where C: RaftTypeConfig
{
    pub(crate) config: &'x mut EngineConfig<C>,
    pub(crate) leader: &'x mut Leader<C, LeaderQuorumSet<C>>,
    pub(crate) state: &'x mut RaftState<C>,
    pub(crate) output: &'x mut EngineOutput<C>,
}

impl<C> LeaderHandler<'_, C>
where C: RaftTypeConfig
{
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
    pub(crate) fn leader_append_entries<I>(&mut self, payloads: I)
    where I: IntoIterator<Item = EntryPayload<C>, IntoIter: ExactSizeIterator> {
        let payloads = payloads.into_iter();

        let log_ids = match self.leader.assign_log_ids(payloads.len()) {
            Some(ids) => ids,
            None => return,
        };

        self.state.extend_log_ids_from_same_leader(log_ids.clone());

        let mut membership_entry = None;
        let entries = Batch::from_iter(payloads.zip(log_ids).map(|(payload, log_id)| {
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
        }));

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
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn send_heartbeat(&mut self) {
        let membership_log_id = self.state.membership_state.effective().log_id();
        let session_id = ReplicationSessionId::new(self.leader.committed_vote.clone(), membership_log_id.clone());

        self.output.push_command(Command::BroadcastHeartbeat { session_id });
    }

    /// Get the log id for a linearizable read.
    ///
    /// See: [Read Operation](crate::docs::protocol::read)
    pub(crate) fn get_read_log_id(&self) -> LogIdOf<C> {
        let committed = self.state.committed().cloned();
        let Some(committed) = committed else {
            return self.leader.noop_log_id.clone();
        };

        // noop log id is the first log this leader proposed.
        std::cmp::max(self.leader.noop_log_id.clone(), committed)
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

    pub(crate) fn replication_handler(&mut self) -> ReplicationHandler<'_, C> {
        ReplicationHandler {
            config: self.config,
            leader: self.leader,
            state: self.state,
            output: self.output,
        }
    }
}
