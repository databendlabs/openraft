use crate::{AppData, AppDataResponse, AppError, RaftNetwork, RaftStorage};
use crate::error::RaftResult;
use crate::raft::{AppendEntriesRequest, AppendEntriesResponse, ConflictOpt, Entry, EntryPayload};
use crate::core::{RaftCore, State, UpdateCurrentLeader};

impl<D: AppData, R: AppDataResponse, E: AppError, N: RaftNetwork<D, E>, S: RaftStorage<D, R, E>> RaftCore<D, R, E, N, S> {
    /// An RPC invoked by the leader to replicate log entries (§5.3); also used as heartbeat (§5.2).
    ///
    /// See `receiver implementation: AppendEntries RPC` in raft-essentials.md in this repo.
    #[tracing::instrument(level="trace", skip(self, msg))]
    pub(super) async fn handle_append_entries_request(&mut self, msg: AppendEntriesRequest<D>) -> RaftResult<AppendEntriesResponse, E> {
        // If message's term is less than most recent term, then we do not honor the request.
        if &msg.term < &self.current_term {
            tracing::trace!({self.current_term, rpc_term=msg.term}, "AppendEntries RPC term is less than current term");
            return Ok(AppendEntriesResponse{term: self.current_term, success: false, conflict_opt: None});
        }

        // Update election timeout.
        self.update_next_election_timeout();

        // Update current term if needed.
        if &self.current_term != &msg.term {
            self.update_current_term(msg.term, None);
            self.save_hard_state().await?;
        }

        // Update current leader if needed.
        if self.current_leader.as_ref() != Some(&msg.leader_id) {
            self.update_current_leader(UpdateCurrentLeader::OtherNode(msg.leader_id));
        }

        // Transition to follower state if needed.
        if !self.target_state.is_follower() && !self.target_state.is_non_voter() {
            self.set_target_state(State::Follower);
        }

        // Apply any outstanding logs to state machine based on `msg.leader_commit`.
        self.commit_index = msg.leader_commit; // The value for `self.commit_index` is only updated here when not the leader.
        if &self.commit_index > &self.last_applied {
            // Fetch the series of entries which must be applied to the state machine, and apply them.
            let entries = self.storage.get_log_entries(self.last_applied + 1, self.commit_index + 1).await.map_err(|err| self.map_fatal_storage_result(err))?;
            self.storage.replicate_to_state_machine(&entries).await.map_err(|err| self.map_fatal_storage_result(err))?;
            if let Some(last_applied_entry) = entries.last() {
                self.last_applied = last_applied_entry.index;
            }

            // Request async compaction, if needed.
            self.trigger_log_compaction_if_needed();
        }

        // If this is just a heartbeat, then respond.
        if msg.entries.len() == 0 {
            return Ok(AppendEntriesResponse{term: self.current_term, success: true, conflict_opt: None});
        }

        // If RPC's `prev_log_index` is 0, or the RPC's previous log info matches the local
        // log info, then replication is g2g.
        let msg_prev_index_is_min = &msg.prev_log_index == &u64::min_value();
        let msg_index_and_term_match = (&msg.prev_log_index == &self.last_log_index) && (&msg.prev_log_term == &self.last_log_term);
        if msg_prev_index_is_min || msg_index_and_term_match {
            self.append_log_entries(&msg.entries).await?;
            self.report_metrics();
            return Ok(AppendEntriesResponse{term: self.current_term, success: true, conflict_opt: None});
        }

        /////////////////////////////////////
        //// Begin Log Consistency Check ////
        tracing::trace!("begin log consistency check");

        // Previous log info doesn't immediately line up, so perform log consistency check and proceed based on its result.
        let entries = self.storage.get_log_entries(msg.prev_log_index, msg.prev_log_index).await.map_err(|err| self.map_fatal_storage_result(err))?;
        let target_entry = match entries.first() {
            Some(target_entry) => target_entry,
            // The target entry was not found. This can only mean that we don't have the
            // specified index yet. Use the last known index & term as a conflict opt.
            None => return Ok(AppendEntriesResponse{
                term: self.current_term, success: false,
                conflict_opt: Some(ConflictOpt{term: self.last_log_term, index: self.last_log_index}),
            }),
        };

        // The target entry was found. Compare its term with target term to ensure everything is consistent.
        if &target_entry.term == &msg.prev_log_term {
            // We've found a point of agreement with the leader. If we have any logs present
            // with an index greater than this, then we must delete them per §5.3.
            if &self.last_log_index > &target_entry.index {
                self.storage.delete_logs_from(target_entry.index + 1, None).await.map_err(|err| self.map_fatal_storage_result(err))?;
            }
        }
        // The target entry does not have the same term. Fetch the last 50 logs, and use the last
        // entry of that payload which is still in the target term for conflict optimization.
        else {
            let start = if &msg.prev_log_index >= &50 { &msg.prev_log_index - 50 } else { 0 };
            let old_entries = self.storage.get_log_entries(start, msg.prev_log_index).await.map_err(|err| self.map_fatal_storage_result(err))?;
            let opt = match old_entries.iter().find(|entry| entry.term == msg.prev_log_term) {
                Some(entry) => Some(ConflictOpt{term: entry.term, index: entry.index}),
                None => Some(ConflictOpt{term: self.last_log_term, index: self.last_log_index}),
            };
            return Ok(AppendEntriesResponse{term: self.current_term, success: false, conflict_opt: opt});
        }

        ///////////////////////////////////
        //// End Log Consistency Check ////
        tracing::trace!("end log consistency check");

        self.append_log_entries(&msg.entries).await?;
        self.report_metrics();
        Ok(AppendEntriesResponse{term: self.current_term, success: true, conflict_opt: None})
    }

    /// Append the given entries to the log.
    ///
    /// Configuration changes are also detected and applied here. See `configuration changes`
    /// in the raft-essentials.md in this repo.
    #[tracing::instrument(level="trace", skip(self, entries))]
    async fn append_log_entries(&mut self, entries: &[Entry<D>]) -> RaftResult<(), E> {
        // Check the given entries for any config changes and take the most recent.
        let last_conf_change = entries.iter()
            .filter_map(|ent| match &ent.payload {
                EntryPayload::ConfigChange(conf) => Some(conf),
                _ => None,
            })
            .last();
        if let Some(conf) = last_conf_change {
            tracing::debug!({membership=?conf}, "applying new membership config received from leader");
            self.update_membership(conf.membership.clone()).await?;
        };

        // Replicate entries to log (same as append, but in follower mode).
        self.storage.replicate_to_log(entries).await.map_err(|err| self.map_fatal_storage_result(err))?;
        if let Some(entry) = entries.last() {
            self.last_log_index = entry.index;
            self.last_log_term = entry.term;
        }
        Ok(())
    }
}
