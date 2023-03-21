use std::marker::PhantomData;

use crate::engine::engine_impl::EngineOutput;
use crate::engine::handler::replication_handler::ReplicationHandler;
use crate::engine::handler::replication_handler::SendNone;
use crate::engine::Command;
use crate::engine::EngineConfig;
use crate::entry::RaftEntry;
use crate::internal_server_state::LeaderQuorumSet;
use crate::leader::Leader;
use crate::raft_state::LogStateReader;
use crate::Node;
use crate::NodeId;
use crate::RaftState;

#[cfg(test)] mod append_entries_test;
#[cfg(test)] mod send_heartbeat_test;

/// Handle leader operations.
///
/// - Append new logs;
/// - Change membership;
/// - etc
pub(crate) struct LeaderHandler<'x, NID, N, Ent>
where
    NID: NodeId,
    N: Node,
    Ent: RaftEntry<NID, N>,
{
    pub(crate) config: &'x mut EngineConfig<NID>,
    pub(crate) leader: &'x mut Leader<NID, LeaderQuorumSet<NID>>,
    pub(crate) state: &'x mut RaftState<NID, N>,
    pub(crate) output: &'x mut EngineOutput<NID, N>,
    pub(crate) _p: PhantomData<Ent>,
}

impl<'x, NID, N, Ent> LeaderHandler<'x, NID, N, Ent>
where
    NID: NodeId,
    N: Node,
    Ent: RaftEntry<NID, N>,
{
    /// Append new log entries by a leader.
    ///
    /// Also Update effective membership if the payload contains
    /// membership config.
    ///
    /// If there is a membership config log entry, the caller has to guarantee the previous one is
    /// committed.
    ///
    /// TODO(xp): metrics flag needs to be dealt with.
    /// TODO(xp): if vote indicates this node is not the leader, refuse append
    #[tracing::instrument(level = "debug", skip(self, entries))]
    pub(crate) fn leader_append_entries(&mut self, entries: &mut [Ent]) {
        let l = entries.len();
        if l == 0 {
            return;
        }

        self.state.assign_log_ids(entries.iter_mut());
        self.state.extend_log_ids_from_same_leader(entries);

        self.output.push_command(Command::AppendInputEntries { range: 0..l });

        // Fast commit:
        // If the cluster has only one voter, then an entry will be committed as soon as it is
        // appended. But if there is a membership log in the middle of the input entries,
        // the condition to commit will change. Thus we have to deal with entries before and
        // after a membership entry differently:
        //
        // When a membership entry is seen, update progress for all former entries.
        // Then upgrade the quorum set for the Progress.
        //
        // E.g., if the input entries are `2..6`, entry 4 changes membership from `a` to `abc`.
        // Then it will output a LeaderCommit command to commit entries `2,3`.
        // ```text
        // 1 2 3 4 5 6
        // a x x a y y
        //       b
        //       c
        // ```
        //
        // If the input entries are `2..6`, entry 4 changes membership from `abc` to `a`.
        // Then it will output a LeaderCommit command to commit entries `2,3,4,5,6`.
        // ```text
        // 1 2 3 4 5 6
        // a x x a y y
        // b
        // c
        // ```

        let mut rh = self.replication_handler();

        for entry in entries.iter() {
            if let Some(m) = entry.get_membership() {
                let log_index = entry.get_log_id().index;

                if log_index > 0 {
                    let prev_log_id = rh.state.get_log_id(log_index - 1);
                    rh.update_local_progress(prev_log_id);
                }

                // since this entry, the condition to commit has been changed.
                rh.append_membership(entry.get_log_id(), m);
            }
        }

        let last_log_id = {
            // Safe unwrap(): entries.len() > 0
            let last = entries.last().unwrap();
            Some(*last.get_log_id())
        };

        rh.update_local_progress(last_log_id);
        rh.initiate_replication(SendNone::False);

        self.output.push_command(Command::MoveInputCursorBy { n: l });
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) fn send_heartbeat(&mut self) -> () {
        let mut rh = self.replication_handler();
        rh.initiate_replication(SendNone::True);
    }

    pub(crate) fn replication_handler(&mut self) -> ReplicationHandler<NID, N> {
        ReplicationHandler {
            config: self.config,
            leader: self.leader,
            state: self.state,
            output: self.output,
        }
    }
}
