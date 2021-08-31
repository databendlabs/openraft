use tracing::Instrument;

use crate::core::RaftCore;
use crate::core::State;
use crate::core::UpdateCurrentLeader;
use crate::error::RaftResult;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendEntriesResponse;
use crate::raft::ConflictOpt;
use crate::raft::Entry;
use crate::raft::EntryPayload;
use crate::AppData;
use crate::AppDataResponse;
use crate::LogId;
use crate::MessageSummary;
use crate::RaftError;
use crate::RaftNetwork;
use crate::RaftStorage;
use crate::Update;

impl<D: AppData, R: AppDataResponse, N: RaftNetwork<D>, S: RaftStorage<D, R>> RaftCore<D, R, N, S> {
    /// An RPC invoked by the leader to replicate log entries (§5.3); also used as heartbeat (§5.2).
    ///
    /// See `receiver implementation: AppendEntries RPC` in raft-essentials.md in this repo.
    #[tracing::instrument(level="trace", skip(self, msg), fields(msg=%msg.summary()))]
    pub(super) async fn handle_append_entries_request(
        &mut self,
        msg: AppendEntriesRequest<D>,
    ) -> RaftResult<AppendEntriesResponse> {
        tracing::debug!(%self.last_log_id);

        let mut msg_entries = msg.entries.as_slice();
        let mut prev_log_id = msg.prev_log_id;

        // If message's term is less than most recent term, then we do not honor the request.
        if msg.term < self.current_term {
            tracing::debug!({self.current_term, rpc_term=msg.term}, "AppendEntries RPC term is less than current term");
            return Ok(AppendEntriesResponse {
                term: self.current_term,
                success: false,
                conflict_opt: None,
            });
        }

        self.update_next_election_timeout(true);

        // Caveat: Because we can not just delete `log[prev_log_id.index..]`, (which results in loss of committed
        // entry), the commit index must be update only after append-entries
        // and must point to a log entry that is consistent to leader.
        // Or there would be chance applying an uncommitted entry:
        //
        // ```
        // R0 1,1  1,2  3,3
        // R1 1,1  1,2  2,3
        // R2 1,1  1,2  3,3
        // ```
        //
        // - R0 to R1 append_entries: entries=[{1,2}], prev_log_id = {1,1}, commit_index = 3
        // - R1 accepted this append_entries request but was not aware of that entry {2,3} is inconsistent to leader.
        //   Then it will update commit_index to 3 and apply {2,3}

        let valid_commit_index = msg_entries.last().map(|x| x.log_id.index).unwrap_or(prev_log_id.index);
        let valid_commit_index = std::cmp::min(msg.leader_commit, valid_commit_index);

        tracing::debug!("start to check and update to latest term/leader");
        {
            let mut report_metrics = false;

            if msg.term > self.current_term {
                self.update_current_term(msg.term, None);
                self.save_hard_state().await?;
                report_metrics = true;
            }

            // Update current leader if needed.
            if self.current_leader.as_ref() != Some(&msg.leader_id) {
                self.update_current_leader(UpdateCurrentLeader::OtherNode(msg.leader_id));
                report_metrics = true;
            }

            if report_metrics {
                self.report_metrics(Update::Ignore);
            }
        }

        // Transition to follower state if needed.
        if !self.target_state.is_follower() && !self.target_state.is_non_voter() {
            self.set_target_state(State::Follower);
        }

        if prev_log_id.index == u64::MIN || prev_log_id == self.last_log_id {
            // Matches! Great!
            return self.append_apply_log_entries(msg_entries, valid_commit_index).await;
        }

        tracing::debug!("begin log consistency check");

        // Lagging too much, let the leader to retry append_entries from my last_log.index
        if self.last_log_id.index < prev_log_id.index {
            let last = self
                .storage
                .try_get_log_entry(self.last_log_id.index)
                .await
                .map_err(|e| self.map_fatal_storage_error(e))?
                .summary();
            tracing::debug!(
                "conflict: last_log_id({}) < prev_log_id({}), conflict = last_log_id: {:?}",
                self.last_log_id,
                prev_log_id,
                last
            );
            return Ok(AppendEntriesResponse {
                term: self.current_term,
                success: false,
                conflict_opt: Some(ConflictOpt {
                    log_id: self.last_log_id,
                }),
            });
        }

        // prev_log_id.index <= last_log_id.index

        // Log entries upto last_applied may be removed.
        // The applied entries are also committed thus always be consistent with the leader.
        // Align the prev_log_id to last_applied.
        let local_prev_log_id = self.earliest_log_id_since(prev_log_id.index).await?;

        if prev_log_id.index < local_prev_log_id.index {
            let distance = local_prev_log_id.index - prev_log_id.index;

            prev_log_id = local_prev_log_id;

            msg_entries = if msg_entries.len() > distance as usize {
                &msg_entries[distance as usize..]
            } else {
                &[]
            };
        }

        // last_applied.index <= prev_log_id.index <= last_log_id.index

        // The target entry was found. Compare its term with target term to ensure everything is consistent.
        if local_prev_log_id == prev_log_id {
            // We've found a point of agreement with the leader. If we have any logs present
            // with an index greater than this, then we must delete them per §5.3.

            if self.last_log_id.index > prev_log_id.index {
                self.delete_inconsistent_log(prev_log_id, msg_entries).await?;
            }
            tracing::debug!("end log consistency check");

            return self.append_apply_log_entries(msg_entries, valid_commit_index).await;
        }

        let last_match = self.last_possible_matched(prev_log_id).await?;

        tracing::debug!("conflict: search for last possible match, conflict = {}", last_match);

        Ok(AppendEntriesResponse {
            term: self.current_term,
            success: false,
            conflict_opt: Some(ConflictOpt { log_id: last_match }),
        })
    }

    #[tracing::instrument(level = "debug", skip(self))]
    async fn earliest_log_id_since(&mut self, index: u64) -> RaftResult<LogId> {
        if index <= self.last_applied.index {
            return Ok(self.last_applied);
        }

        // last_applied.index < prev_log_id.index <= last_log_id.index

        let x = self.storage.get_log_entries(index..=index).await.map_err(|err| self.map_fatal_storage_error(err))?;

        let entry = x
            .first()
            .ok_or_else(|| self.map_fatal_storage_error(anyhow::anyhow!("log entry not found at: {}", index)))?;

        Ok(entry.log_id)
    }

    #[tracing::instrument(level="debug", skip(self, msg_entries), fields(msg_entries=%msg_entries.summary()))]
    async fn delete_inconsistent_log(&mut self, prev_log_id: LogId, msg_entries: &[Entry<D>]) -> RaftResult<()> {
        // Caveat: Deleting then appending entries are not atomic, thus deleting consistent entries may cause loss of
        // committed logs.
        //
        // E.g., the logs are as following and R1 now is the leader:
        //
        // ```
        // R1 1,1  1,2  1,3
        // R2 1,1  1,2
        // R3
        // ```
        // When the following steps take place, committed entry `{1,2}` is lost:
        //
        // - R1 to R2: append_entries(entries=[{1,2}, {1,3}], prev_log_id={1,1})
        // - R2 deletes 1,2
        // - R2 crash
        // - R2 elected as leader and only see 1,1; the committed entry 1,2 is lost.
        //
        // **The safe way is to skip every entry that present in append_entries message then delete only the
        // inconsistent entries**.

        let end = std::cmp::min(prev_log_id.index + msg_entries.len() as u64, self.last_log_id.index + 1);

        tracing::debug!(
            "find and delete inconsistent log entries [{}, {}), last_log_id: {}, entries: {}",
            prev_log_id,
            end,
            self.last_log_id,
            msg_entries.summary()
        );

        let entries = self
            .storage
            .get_log_entries(prev_log_id.index..end)
            .await
            .map_err(|err| self.map_fatal_storage_error(err))?;

        for (i, ent) in entries.iter().enumerate() {
            if ent.log_id.term != msg_entries[i].log_id.term {
                tracing::debug!("delete inconsistent log entries from: {}", ent.log_id);

                self.storage
                    .delete_logs_from(ent.log_id.index..)
                    .await
                    .map_err(|err| self.map_fatal_storage_error(err))?;

                let membership =
                    self.storage.get_membership_config().await.map_err(|err| self.map_fatal_storage_error(err))?;

                self.update_membership(membership)?;

                break;
            }
        }
        Ok(())
    }

    /// Walks backward 50 entries to find the last log entry that has the same `term` as `prev_log_id`, which is
    /// consistent to the leader. Further replication will use this log id as `prev_log_id` to sync log.
    #[tracing::instrument(level = "debug", skip(self))]
    async fn last_possible_matched(&mut self, prev_log_id: LogId) -> RaftResult<LogId> {
        let start = prev_log_id.index.saturating_sub(50);

        if start < self.last_applied.index {
            // The applied log is always consistent to leader.
            return Ok(self.last_applied);
        }

        if start == 0 {
            // A simple way is to sync from the beginning.
            return Ok(LogId { term: 0, index: 0 });
        }

        let old_entries = self
            .storage
            .get_log_entries(start..=prev_log_id.index)
            .await
            .map_err(|err| self.map_fatal_storage_error(err))?;

        let last_matched = old_entries.iter().rev().find(|entry| entry.log_id.term == prev_log_id.term);

        let first = old_entries.first().map(|x| x.log_id).unwrap();

        let last_matched = last_matched.map(|x| x.log_id).unwrap_or_else(|| first);
        Ok(last_matched)
    }

    #[tracing::instrument(level="debug", skip(self, entries), fields(entries=%entries.summary()))]
    async fn append_apply_log_entries(
        &mut self,
        entries: &[Entry<D>],
        commit_index: u64,
    ) -> RaftResult<AppendEntriesResponse> {
        if !entries.is_empty() {
            self.append_log_entries(entries).await?;
        }

        self.commit_index = commit_index;

        self.replicate_to_state_machine_if_needed().await?;

        self.report_metrics(Update::Ignore);

        Ok(AppendEntriesResponse {
            term: self.current_term,
            success: true,
            conflict_opt: None,
        })
    }

    /// Append the given entries to the log.
    ///
    /// Configuration changes are also detected and applied here. See `configuration changes`
    /// in the raft-essentials.md in this repo.
    #[tracing::instrument(level = "trace", skip(self, entries))]
    async fn append_log_entries(&mut self, entries: &[Entry<D>]) -> RaftResult<()> {
        // Check the given entries for any config changes and take the most recent.
        let last_conf_change = entries
            .iter()
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
        let entry_refs = entries.iter().collect::<Vec<_>>();
        self.storage.append_to_log(&entry_refs).await.map_err(|err| self.map_fatal_storage_error(err))?;
        if let Some(entry) = entries.last() {
            self.last_log_id = entry.log_id;
        }
        Ok(())
    }

    /// Replicate any outstanding entries to the state machine for which it is safe to do so.
    ///
    /// Very importantly, this routine must not block the main control loop main task, else it
    /// may cause the Raft leader to timeout the requests to this node.
    #[tracing::instrument(level = "trace", skip(self))]
    async fn replicate_to_state_machine_if_needed(&mut self) -> Result<(), RaftError> {
        tracing::debug!("replicate_to_sm_if_needed: last_applied: {}", self.last_applied,);

        // Perform initial replication to state machine if needed.
        if !self.has_completed_initial_replication_to_sm {
            // Optimistic update, as failures will cause shutdown.
            self.has_completed_initial_replication_to_sm = true;
            self.initial_replicate_to_state_machine().await;
            return Ok(());
        }

        // If we already have an active replication task, then do nothing.
        if !self.replicate_to_sm_handle.is_empty() {
            tracing::debug!("replicate_to_sm_handle is not empty, return");
            return Ok(());
        }

        // If we don't have any new entries to replicate, then do nothing.
        if self.commit_index <= self.last_applied.index {
            tracing::debug!(
                "commit_index({}) <= last_applied({}), return",
                self.commit_index,
                self.last_applied
            );
            return Ok(());
        }

        // Drain entries from the beginning of the cache up to commit index.

        // TODO(xp): logs in storage must be consecutive.
        let entries = self
            .storage
            .get_log_entries(self.last_applied.index + 1..=self.commit_index)
            .await
            .map_err(|e| self.map_fatal_storage_error(e))?;

        let last_log_id = entries.last().map(|x| x.log_id);

        tracing::debug!("entries: {:?}", entries.iter().map(|x| x.log_id).collect::<Vec<_>>());
        tracing::debug!(?last_log_id);

        // If we have no data entries to apply, then do nothing.
        if entries.is_empty() {
            if let Some(log_id) = last_log_id {
                self.last_applied = log_id;
                self.report_metrics(Update::Ignore);
            }
            tracing::debug!("entries is empty, return");
            return Ok(());
        }

        // Spawn task to replicate these entries to the state machine.
        // Linearizability is guaranteed by `replicate_to_sm_handle`, which is the mechanism used
        // to ensure that only a single task can replicate data to the state machine, and that is
        // owned by a single task, not shared between multiple threads/tasks.
        let storage = self.storage.clone();
        let handle = tokio::spawn(
            async move {
                // Create a new vector of references to the entries data ... might have to change this
                // interface a bit before 1.0.
                let entries_refs: Vec<_> = entries.iter().collect();
                storage.apply_to_state_machine(&entries_refs).await?;
                Ok(last_log_id)
            }
            .instrument(tracing::debug_span!("spawn")),
        );
        self.replicate_to_sm_handle.push(handle);

        Ok(())
    }

    /// Perform an initial replication of outstanding entries to the state machine.
    ///
    /// This will only be executed once, and only in response to its first payload of entries
    /// from the AppendEntries RPC handler.
    #[tracing::instrument(level = "trace", skip(self))]
    async fn initial_replicate_to_state_machine(&mut self) {
        let stop = std::cmp::min(self.commit_index, self.last_log_id.index) + 1;
        let start = self.last_applied.index + 1;
        let storage = self.storage.clone();

        // If we already have an active replication task, then do nothing.
        if !self.replicate_to_sm_handle.is_empty() {
            return;
        }

        tracing::debug!(start, stop, self.commit_index, %self.last_log_id, "start stop");

        // when self.commit_index is not initialized, e.g. the first heartbeat from leader always has a commit_index to
        // be 0, because the leader needs one round of heartbeat to find out the commit index.
        if start >= stop {
            return;
        }

        // Fetch the series of entries which must be applied to the state machine, then apply them.
        let handle = tokio::spawn(
            async move {
                let mut new_last_applied: Option<LogId> = None;
                let entries = storage.get_log_entries(start..stop).await?;
                if let Some(entry) = entries.last() {
                    new_last_applied = Some(entry.log_id);
                }
                let data_entries: Vec<_> = entries.iter().collect();
                if data_entries.is_empty() {
                    return Ok(new_last_applied);
                }
                storage.apply_to_state_machine(&data_entries).await?;
                Ok(new_last_applied)
            }
            .instrument(tracing::debug_span!("spawn-init-replicate-to-sm")),
        );
        self.replicate_to_sm_handle.push(handle);
    }
}
