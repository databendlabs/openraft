use crate::core::apply_to_state_machine;
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
        tracing::debug!(%self.last_log_id, %self.last_applied);

        let msg_entries = msg.entries.as_slice();
        let prev_log_id = msg.prev_log_id;

        if !msg_entries.is_empty() {
            assert_eq!(prev_log_id.index + 1, msg_entries.first().unwrap().log_id.index);
        }

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

        tracing::debug!("begin log consistency check");

        // There are 5 cases a prev_log_id could have:
        // prev_log_id: 0       1        2            3           4           5
        //              +----------------+------------------------+
        //              ` 0              ` last_applied           ` last_log_id

        // Case 0: prev == 0
        if prev_log_id.index == u64::MIN {
            if self.last_log_id.index == prev_log_id.index {
                // Matches! Great!
                tracing::debug!("append-entries case-0 OK: prev_log_id({}) == 0", prev_log_id,);
                return self.append_apply_log_entries(prev_log_id.index + 1, msg_entries, valid_commit_index).await;
            } else {
                tracing::debug!(
                    "append-entries case-0: prev_log_id({}) is 0, conflict = last_log_id: {:?}",
                    prev_log_id,
                    self.last_log_id
                );

                return Ok(AppendEntriesResponse {
                    term: self.current_term,
                    success: false,
                    conflict_opt: Some(ConflictOpt {
                        log_id: self.last_log_id,
                    }),
                });
            }
        }

        // Case 1: 0 < prev < last_applied
        if prev_log_id.index < self.last_applied.index {
            tracing::debug!(
                "append-entries case-1: prev_log_id({}) < last_applied({}), conflict = last_log_id: {:?}",
                prev_log_id,
                self.last_applied,
                self.last_log_id
            );

            return Ok(AppendEntriesResponse {
                term: self.current_term,
                success: false,
                conflict_opt: Some(ConflictOpt {
                    log_id: self.last_log_id,
                }),
            });
        }

        // Case 2: prev == last_applied
        if prev_log_id.index == self.last_applied.index {
            // The applied entries are also committed thus always be consistent with the leader.

            assert_eq!(prev_log_id, self.last_applied);

            tracing::debug!(
                "append-entries case-2: prev_log_id({}) == last_applied({})",
                prev_log_id,
                self.last_applied,
            );

            return self.append_apply_log_entries(prev_log_id.index + 1, msg_entries, valid_commit_index).await;
        }

        // Case 3, 4: last_applied < prev <= last_log_id
        if prev_log_id.index <= self.last_log_id.index {
            tracing::debug!(
                "append-entries case-3,4: prev_log_id({}) <= last_log_id({})",
                prev_log_id,
                self.last_applied,
            );

            let local = self.get_log_id(prev_log_id.index).await?;

            if prev_log_id == local {
                return self.append_apply_log_entries(prev_log_id.index + 1, msg_entries, valid_commit_index).await;
            } else {
                if prev_log_id.index <= self.last_log_id.index {
                    self.delete_logs(prev_log_id.index).await?;
                }

                let log_id = self.get_older_log_id(prev_log_id).await?;

                tracing::debug!(
                    "append-entries case-3,4: prev_log_id({}) <= last_log_id({}), conflict = {}",
                    prev_log_id,
                    self.last_applied,
                    log_id
                );

                return Ok(AppendEntriesResponse {
                    term: self.current_term,
                    success: false,
                    conflict_opt: Some(ConflictOpt { log_id }),
                });
            }
        }

        // Case 5: prev > last_log_id

        tracing::debug!(
                "append-entries case-5 advanced last_log_id: prev_log_id({}) > last_log_id({}), conflict = last_log_id: {:?}",
                prev_log_id,
                self.last_log_id,
                self.last_log_id
            );

        assert!(prev_log_id.index > self.last_log_id.index);

        Ok(AppendEntriesResponse {
            term: self.current_term,
            success: false,
            conflict_opt: Some(ConflictOpt {
                log_id: self.last_log_id,
            }),
        })
    }

    #[tracing::instrument(level = "debug", skip(self))]
    async fn delete_logs(&mut self, start: u64) -> RaftResult<()> {
        self.storage.delete_logs_from(start..).await.map_err(|err| self.map_storage_error(err))?;

        self.last_log_id = self.get_log_id(start - 1).await?;

        let membership = self.storage.get_membership_config().await.map_err(|err| self.map_storage_error(err))?;

        self.update_membership(membership)?;

        Ok(())
    }

    #[tracing::instrument(level = "debug", skip(self))]
    async fn get_log_id(&mut self, index: u64) -> RaftResult<LogId> {
        assert!(index >= self.last_applied.index);

        if index == self.last_applied.index {
            return Ok(self.last_applied);
        }

        let entries = self.storage.get_log_entries(index..=index).await.map_err(|err| self.map_storage_error(err))?;

        let entry = entries
            .first()
            .ok_or_else(|| self.map_fatal_storage_error(anyhow::anyhow!("log entry not found at: {}", index)))?;

        Ok(entry.log_id)
    }

    /// Skip log entries that have the same term as the entries the leader sent.
    /// Delete entries since the first mismatching entry from local storage.
    /// Returns a slice of entries that are not in local storage.
    ///
    /// Caveat: Deleting then appending entries are not atomic, thus deleting consistent entries may cause loss of
    /// committed logs.
    ///
    /// E.g., the entries are as following and R1 now is the leader:
    ///
    /// ```text
    /// R1 1,1  1,2  1,3
    /// R2 1,1  1,2
    /// R3
    /// ```
    ///
    /// When the following steps take place, committed entry `{1,2}` is lost:
    ///
    /// - R1 to R2: `append_entries(entries=[{1,2}, {1,3}], prev_log_id={1,1})`
    /// - R2 deletes `{1,2}`
    /// - R2 crash
    /// - R2 elected as leader and only see 1,1; the committed entry 1,2 is lost.
    ///
    /// **The safe way is to skip every entry that present in append-entries message then delete only the
    /// inconsistent entries**.
    ///
    /// Why need to delete:
    ///
    /// The following diagram shows only log term.
    ///
    /// ```text
    /// R1 5
    /// R2 5
    /// R3 5 3 3
    /// R4
    /// R5 2 4 4
    /// ```
    ///
    /// If log 5 is committed by R1, and log 3 is not removed, R5 in future could become a new leader and overrides log
    /// 5 on R3.
    #[tracing::instrument(level="debug", skip(self, msg_entries), fields(msg_entries=%msg_entries.summary()))]
    async fn delete_inconsistent_log<'s, 'e>(
        &'s mut self,
        index: u64,
        msg_entries: &'e [Entry<D>],
    ) -> RaftResult<&'e [Entry<D>]> {
        let end = std::cmp::min(index + msg_entries.len() as u64, self.last_log_id.index + 1);

        if index == end {
            return Ok(msg_entries);
        }

        tracing::debug!(
            "find and delete inconsistent log entries [{}, {}), last_log_id: {}, entries: {}",
            index,
            end,
            self.last_log_id,
            msg_entries.summary()
        );

        let entries = self.storage.get_log_entries(index..end).await.map_err(|err| self.map_storage_error(err))?;

        for (i, ent) in entries.iter().enumerate() {
            assert_eq!(msg_entries[i].log_id.index, ent.log_id.index);

            if ent.log_id.term != msg_entries[i].log_id.term {
                tracing::debug!(
                    "delete inconsistent log entries from: {}-th msg.entries: {}",
                    i,
                    ent.log_id
                );

                self.delete_logs(ent.log_id.index).await?;

                return Ok(&msg_entries[i..]);
            }
        }
        Ok(&[])
    }

    /// Walks at most 50 entries backward to get an entry as the `prev_log_id` for next append-entries request.
    #[tracing::instrument(level = "debug", skip(self))]
    async fn get_older_log_id(&mut self, prev_log_id: LogId) -> RaftResult<LogId> {
        let start = prev_log_id.index.saturating_sub(50);

        if start <= self.last_applied.index {
            // The applied log is always consistent to any leader.
            return Ok(self.last_applied);
        }

        if start == 0 {
            // A simple way is to sync from the beginning.
            return Ok(LogId { term: 0, index: 0 });
        }

        let entries = self.storage.get_log_entries(start..=start).await.map_err(|err| self.map_storage_error(err))?;

        let log_id = entries.first().unwrap().log_id;

        Ok(log_id)
    }

    #[tracing::instrument(level="debug", skip(self, entries), fields(entries=%entries.summary()))]
    async fn append_apply_log_entries(
        &mut self,
        index: u64,
        entries: &[Entry<D>],
        commit_index: u64,
    ) -> RaftResult<AppendEntriesResponse> {
        // Before appending, if an entry overrides an inconsistent one, the entries after it must be deleted first.
        let entries = self.delete_inconsistent_log(index, entries).await?;

        self.append_log_entries(entries).await?;

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
    #[tracing::instrument(level = "trace", skip(self, entries), fields(entries=%entries.summary()))]
    async fn append_log_entries(&mut self, entries: &[Entry<D>]) -> RaftResult<()> {
        if entries.is_empty() {
            return Ok(());
        }

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
        self.storage.append_to_log(&entry_refs).await.map_err(|err| self.map_storage_error(err))?;
        if let Some(entry) = entries.last() {
            self.last_log_id = entry.log_id;
        }
        Ok(())
    }

    /// Replicate any outstanding entries to the state machine for which it is safe to do so.
    ///
    /// Very importantly, this routine must not block the main control loop main task, else it
    /// may cause the Raft leader to timeout the requests to this node.
    #[tracing::instrument(level = "debug", skip(self))]
    async fn replicate_to_state_machine_if_needed(&mut self) -> Result<(), RaftError> {
        tracing::debug!("replicate_to_sm_if_needed: last_applied: {}", self.last_applied,);

        // Perform initial replication to state machine if needed.
        if !self.has_completed_initial_replication_to_sm {
            // Optimistic update, as failures will cause shutdown.
            self.has_completed_initial_replication_to_sm = true;
            self.initial_replicate_to_state_machine().await?;
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

        let entries = self
            .storage
            .get_log_entries(self.last_applied.index + 1..=self.commit_index)
            .await
            .map_err(|e| self.map_storage_error(e))?;

        let last_log_id = entries.last().map(|x| x.log_id).unwrap();

        tracing::debug!("entries: {}", entries.as_slice().summary());
        tracing::debug!(?last_log_id);

        let entries_refs: Vec<_> = entries.iter().collect();

        apply_to_state_machine(self.storage.clone(), &entries_refs, self.config.max_applied_log_to_keep)
            .await
            .map_err(|e| self.map_storage_error(e))?;

        self.last_applied = last_log_id;

        self.report_metrics(Update::Ignore);
        self.trigger_log_compaction_if_needed(false);

        Ok(())
    }

    /// Perform an initial replication of outstanding entries to the state machine.
    ///
    /// This will only be executed once, and only in response to its first payload of entries
    /// from the AppendEntries RPC handler.
    #[tracing::instrument(level = "debug", skip(self))]
    async fn initial_replicate_to_state_machine(&mut self) -> Result<(), RaftError> {
        let stop = std::cmp::min(self.commit_index, self.last_log_id.index) + 1;
        let start = self.last_applied.index + 1;
        let storage = self.storage.clone();

        tracing::debug!(start, stop, self.commit_index, %self.last_log_id, "start stop");

        // when self.commit_index is not initialized, e.g. the first heartbeat from leader always has a commit_index to
        // be 0, because the leader needs one round of heartbeat to find out the commit index.
        if start >= stop {
            return Ok(());
        }

        // Fetch the series of entries which must be applied to the state machine, then apply them.

        let entries = storage.get_log_entries(start..stop).await.map_err(|e| self.map_storage_error(e))?;

        let new_last_applied = entries.last().unwrap();

        let data_entries: Vec<_> = entries.iter().collect();

        apply_to_state_machine(storage, &data_entries, self.config.max_applied_log_to_keep)
            .await
            .map_err(|e| self.map_storage_error(e))?;

        self.last_applied = new_last_applied.log_id;
        self.report_metrics(Update::Ignore);
        self.trigger_log_compaction_if_needed(false);

        Ok(())
    }
}
