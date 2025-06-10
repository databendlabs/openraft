use crate::engine::handler::replication_handler::ReplicationHandler;
use crate::engine::handler::replication_handler::SendNone;
use crate::engine::Command;
use crate::engine::EngineConfig;
use crate::engine::EngineOutput;
use crate::entry::RaftPayload;
use crate::proposer::Leader;
use crate::proposer::LeaderQuorumSet;
use crate::raft_state::LogStateReader;
use crate::type_config::alias::LogIdOf;
use crate::AsyncRuntime;
use crate::RaftLogId;
use crate::RaftState;
use crate::RaftTypeConfig;

#[cfg(test)]
mod append_entries_test;
#[cfg(test)]
mod send_heartbeat_test;

/// Handle leader operations.
///
/// - Append new logs;
/// - Change membership;
/// - etc
pub(crate) struct LeaderHandler<'x, C>
where C: RaftTypeConfig
{
    pub(crate) config: &'x mut EngineConfig<C::NodeId>,
    pub(crate) leader: &'x mut Leader<C, LeaderQuorumSet<C::NodeId>>,
    pub(crate) state: &'x mut RaftState<C::NodeId, C::Node, <C::AsyncRuntime as AsyncRuntime>::Instant>,
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
    #[tracing::instrument(level = "debug", skip(self, entries))]
    pub(crate) fn leader_append_entries(&mut self, mut entries: Vec<C::Entry>) {
        let l = entries.len();
        if l == 0 {
            return;
        }

        self.leader.assign_log_ids(&mut entries);

        self.state.extend_log_ids_from_same_leader(&entries);

        let mut membership_entry = None;
        for entry in entries.iter() {
            if let Some(m) = entry.get_membership() {
                debug_assert!(
                    membership_entry.is_none(),
                    "only one membership entry is allowed in a batch"
                );
                membership_entry = Some((entry.get_log_id().clone(), m.clone()));
            }
        }

        // TODO: In future implementations with asynchronous IO,
        //       ensure logs are not written until the vote is committed
        //       to maintain consistency.
        //       ---
        //       Currently, IO requests to `RaftLogStorage` are executed
        //       within the `RaftCore` task. This means an `AppendLog` request
        //       won't be submitted to `RaftLogStorage` until `save_vote()` completes,
        //       which ensures consistency.
        //       ---
        //       However, when `RaftLogStorage` is moved to a separate task,
        //       `RaftCore` will communicate with `RaftLogStorage` via a channel.
        //       This change could result in `AppendLog` requests being submitted
        //       before the previous `save_vote()` request is finished.
        //       ---
        //       This scenario creates a risk where a log entry becomes visible and
        //       is replicated by `ReplicationCore` to other nodes before the vote
        //       is flushed to disk. If the vote isn't flushed and the server restarts,
        //       the vote could revert to a previous state. This could allow a new leader
        //       to be elected with a smaller vote (term), breaking consistency.
        self.output.push_command(Command::AppendInputEntries {
            // A leader should always use the leader's vote.
            // It is allowed to be different from local vote.
            vote: self.leader.vote.clone(),
            entries,
        });

        let mut rh = self.replication_handler();

        // Since this entry, the condition to commit has been changed.
        // But we only need to commit in the new membership config.
        // Because any quorum in the new one intersect with one in the previous membership config.
        if let Some((log_id, m)) = membership_entry {
            rh.append_membership(&log_id, &m);
        }

        rh.initiate_replication(SendNone::False);
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn send_heartbeat(&mut self) {
        let mut rh = self.replication_handler();
        rh.initiate_replication(SendNone::True);
    }

    /// Get the log id for a linearizable read.
    ///
    /// See: [Read Operation](crate::docs::protocol::read)
    pub(crate) fn get_read_log_id(&self) -> Option<LogIdOf<C>> {
        let committed = self.state.committed().cloned();
        // noop log id is the first log this leader proposed.
        std::cmp::max(self.leader.noop_log_id.clone(), committed)
    }

    pub(crate) fn replication_handler(&mut self) -> ReplicationHandler<C> {
        ReplicationHandler {
            config: self.config,
            leader: self.leader,
            state: self.state,
            output: self.output,
        }
    }
}
