use crate::{AppData, AppDataResponse, RaftNetwork, RaftStorage};
use crate::error::RaftResult;
use crate::raft::{AppendEntriesRequest, AppendEntriesResponse, ConflictOpt, Entry, EntryPayload};
use crate::core::{RaftCore, State, UpdateCurrentLeader};

impl<D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> RaftCore<D, R, N, S> {
    /// An RPC invoked by the leader to replicate log entries (§5.3); also used as heartbeat (§5.2).
    ///
    /// See `receiver implementation: AppendEntries RPC` in raft-essentials.md in this repo.
    #[tracing::instrument(
        level="trace", skip(self, msg),
        fields(term=msg.term, leader_id=msg.leader_id, prev_log_index=msg.prev_log_index, prev_log_term=msg.prev_log_term, leader_commit=msg.leader_commit),
    )]
    pub(super) async fn handle_append_entries_request(&mut self, msg: AppendEntriesRequest<D>) -> RaftResult<AppendEntriesResponse> {
        // If message's term is less than most recent term, then we do not honor the request.
        if msg.term < self.current_term {
            tracing::trace!({self.current_term, rpc_term=msg.term}, "AppendEntries RPC term is less than current term");
            return Ok(AppendEntriesResponse{term: self.current_term, success: false, conflict_opt: None});
        }

        // Update election timeout.
        self.update_next_election_timeout();
        let mut report_metrics = false;
        self.commit_index = msg.leader_commit; // The value for `self.commit_index` is only updated here when not the leader.

        // Update current term if needed.
        if self.current_term != msg.term {
            self.update_current_term(msg.term, None);
            self.save_hard_state().await?;
            report_metrics = true;
        }

        // Update current leader if needed.
        if self.current_leader.as_ref() != Some(&msg.leader_id) {
            self.update_current_leader(UpdateCurrentLeader::OtherNode(msg.leader_id));
            report_metrics = true;
        }

        // Transition to follower state if needed.
        if !self.target_state.is_follower() && !self.target_state.is_non_voter() {
            self.set_target_state(State::Follower);
        }

        // If RPC's `prev_log_index` is 0, or the RPC's previous log info matches the local
        // log info, then replication is g2g.
        let msg_prev_index_is_min = msg.prev_log_index == u64::min_value();
        let msg_index_and_term_match = (msg.prev_log_index == self.last_log_index) && (msg.prev_log_term == self.last_log_term);
        if msg_prev_index_is_min || msg_index_and_term_match {
            // If this is just a heartbeat, then respond.
            if msg.entries.is_empty() {
                self.replicate_to_state_machine_if_needed(&mut report_metrics).await?;
                if report_metrics {
                    self.report_metrics();
                }
                return Ok(AppendEntriesResponse{term: self.current_term, success: true, conflict_opt: None});
            }

            // Else, append log entries.
            self.append_log_entries(&msg.entries).await?;
            self.replicate_to_state_machine_if_needed(&mut report_metrics).await?;
            if report_metrics {
                self.report_metrics();
            }
            return Ok(AppendEntriesResponse{term: self.current_term, success: true, conflict_opt: None});
        }

        /////////////////////////////////////
        //// Begin Log Consistency Check ////
        tracing::trace!("begin log consistency check");

        // Previous log info doesn't immediately line up, so perform log consistency check and proceed based on its result.
        let entries = self.storage.get_log_entries(msg.prev_log_index, msg.prev_log_index).await.map_err(|err| self.map_fatal_storage_error(err))?;
        let target_entry = match entries.first() {
            Some(target_entry) => target_entry,
            // The target entry was not found. This can only mean that we don't have the
            // specified index yet. Use the last known index & term as a conflict opt.
            None => {
                if report_metrics {
                    self.report_metrics();
                }
                return Ok(AppendEntriesResponse{
                    term: self.current_term, success: false,
                    conflict_opt: Some(ConflictOpt{term: self.last_log_term, index: self.last_log_index}),
                });
            }
        };

        // The target entry was found. Compare its term with target term to ensure everything is consistent.
        if target_entry.term == msg.prev_log_term {
            // We've found a point of agreement with the leader. If we have any logs present
            // with an index greater than this, then we must delete them per §5.3.
            if self.last_log_index > target_entry.index {
                self.storage.delete_logs_from(target_entry.index + 1, None).await.map_err(|err| self.map_fatal_storage_error(err))?;
                let membership = self.storage.get_membership_config().await.map_err(|err| self.map_fatal_storage_error(err))?;
                self.update_membership(membership)?;
            }
        }
        // The target entry does not have the same term. Fetch the last 50 logs, and use the last
        // entry of that payload which is still in the target term for conflict optimization.
        else {
            let start = if msg.prev_log_index >= 50 { msg.prev_log_index - 50 } else { 0 };
            let old_entries = self.storage.get_log_entries(start, msg.prev_log_index).await.map_err(|err| self.map_fatal_storage_error(err))?;
            let opt = match old_entries.iter().find(|entry| entry.term == msg.prev_log_term) {
                Some(entry) => Some(ConflictOpt{term: entry.term, index: entry.index}),
                None => Some(ConflictOpt{term: self.last_log_term, index: self.last_log_index}),
            };
            if report_metrics {
                self.report_metrics();
            }
            return Ok(AppendEntriesResponse{term: self.current_term, success: false, conflict_opt: opt});
        }

        ///////////////////////////////////
        //// End Log Consistency Check ////
        tracing::trace!("end log consistency check");

        self.append_log_entries(&msg.entries).await?;
        self.replicate_to_state_machine_if_needed(&mut report_metrics).await?;
        if report_metrics {
            self.report_metrics();
        }
        Ok(AppendEntriesResponse{term: self.current_term, success: true, conflict_opt: None})
    }

    /// Append the given entries to the log.
    ///
    /// Configuration changes are also detected and applied here. See `configuration changes`
    /// in the raft-essentials.md in this repo.
    #[tracing::instrument(level="trace", skip(self, entries))]
    async fn append_log_entries(&mut self, entries: &[Entry<D>]) -> RaftResult<()> {
        // Check the given entries for any config changes and take the most recent.
        let last_conf_change = entries.iter()
            .filter_map(|ent| match &ent.payload {
                EntryPayload::ConfigChange(conf) => Some(conf),
                _ => None,
            })
            .last();
        if let Some(conf) = last_conf_change {
            tracing::debug!({membership=?conf}, "applying new membership config received from leader");
            self.update_membership(conf.membership.clone())?;
        };

        // Replicate entries to log (same as append, but in follower mode).
        self.storage.replicate_to_log(entries).await.map_err(|err| self.map_fatal_storage_error(err))?;
        if let Some(entry) = entries.last() {
            self.last_log_index = entry.index;
            self.last_log_term = entry.term;
        }
        Ok(())
    }

    /// Replicate outstanding logs to the state machine if needed.
    #[tracing::instrument(level="trace", skip(self, report_metrics))]
    async fn replicate_to_state_machine_if_needed(&mut self, report_metrics: &mut bool) -> RaftResult<()> {
        if self.commit_index > self.last_applied {
            // Fetch the series of entries which must be applied to the state machine, and apply them.
            let stop = std::cmp::min(self.commit_index, self.last_log_index) + 1;
            let entries = self.storage.get_log_entries(self.last_applied + 1, stop).await.map_err(|err| self.map_fatal_storage_error(err))?;
            if let Some(entry) = entries.last() {
                self.last_applied = entry.index;
                *report_metrics = true;
            }
            let data_entries: Vec<_> = entries.iter()
                .filter_map(|entry| match &entry.payload {
                    EntryPayload::Normal(inner) => Some((&entry.index, &inner.data)),
                    _ => None,
                })
                .collect();
            if data_entries.is_empty() {
                return Ok(());
            }
            self.storage.replicate_to_state_machine(&data_entries).await.map_err(|err| self.map_fatal_storage_error(err))?;

            // Request async compaction, if needed.
            self.trigger_log_compaction_if_needed();
        }
        Ok(())
    }
}
