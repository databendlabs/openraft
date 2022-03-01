use crate::core::apply_to_state_machine;
use crate::core::RaftCore;
use crate::core::State;
use crate::error::AppendEntriesError;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendEntriesResponse;
use crate::raft::Entry;
use crate::raft::EntryPayload;
use crate::raft_types::LogIdOptionExt;
use crate::EffectiveMembership;
use crate::LogId;
use crate::MessageSummary;
use crate::RaftNetworkFactory;
use crate::RaftStorage;
use crate::RaftTypeConfig;
use crate::StorageError;
use crate::Update;

impl<C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> RaftCore<C, N, S> {
    /// An RPC invoked by the leader to replicate log entries (§5.3); also used as heartbeat (§5.2).
    ///
    /// See `receiver implementation: AppendEntries RPC` in raft-essentials.md in this repo.
    #[tracing::instrument(level = "debug", skip(self, req))]
    pub(super) async fn handle_append_entries_request(
        &mut self,
        req: AppendEntriesRequest<C>,
    ) -> Result<AppendEntriesResponse<C>, AppendEntriesError<C>> {
        tracing::debug!(last_log_id=?self.last_log_id, ?self.last_applied, msg=%req.summary(), "handle_append_entries_request");

        let msg_entries = req.entries.as_slice();

        // Partial order compare: smaller than or incomparable
        if req.vote < self.vote {
            tracing::debug!(?self.vote, %req.vote, "AppendEntries RPC term is less than current term");

            return Ok(AppendEntriesResponse {
                vote: self.vote,
                success: false,
                conflict: false,
            });
        }

        self.update_next_election_timeout(true);

        tracing::debug!("start to check and update to latest term/leader");
        if req.vote > self.vote {
            self.vote = req.vote;
            self.save_vote().await?;

            // If not follower, become follower.
            if !self.target_state.is_follower() && !self.target_state.is_learner() {
                self.set_target_state(State::Follower); // State update will emit metrics.
            }

            self.report_metrics(Update::AsIs);
        }

        // Caveat: [commit-index must not advance the last known consistent log](https://datafuselabs.github.io/openraft/replication.html#caveat-commit-index-must-not-advance-the-last-known-consistent-log)

        // TODO(xp): cleanup commit index at sender side.
        let valid_commit_index = msg_entries.last().map(|x| Some(x.log_id)).unwrap_or_else(|| req.prev_log_id);
        let valid_committed = std::cmp::min(req.leader_commit, valid_commit_index);

        tracing::debug!("begin log consistency check");

        // There are 5 cases a prev_log_id could have:
        // prev_log_id: 0       1        2            3           4           5
        //              +----------------+------------------------+
        //              ` 0              ` last_applied           ` last_log_id

        let res = self.append_apply_log_entries(req.prev_log_id, msg_entries, valid_committed).await?;

        Ok(res)
    }

    #[tracing::instrument(level = "debug", skip(self))]
    async fn delete_conflict_logs_since(&mut self, start: LogId<C>) -> Result<(), StorageError<C>> {
        self.storage.delete_conflict_logs_since(start).await?;

        self.last_log_id = self.storage.get_log_state().await?.last_log_id;

        // TODO(xp): get_membership() should have a defensive check to ensure it always returns Some() if node is
        //           initialized. Because a node always commit a membership log as the first log entry.
        let membership = self.storage.get_membership().await?;

        // TODO(xp): This is a dirty patch:
        //           When a node starts in a single-node mode, it does not append an initial log
        //           but instead depends on storage.get_membership() to return a default one.
        //           It would be better a node always append an initial log entry.
        let membership = membership.unwrap_or_else(|| EffectiveMembership::new_initial(self.id));

        self.update_membership(membership);

        tracing::debug!("Done update membership");

        Ok(())
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
    /// If log 5 is committed by R1, and log 3 is not removeC5 in future could become a new leader and overrides log
    /// 5 on R3.
    #[tracing::instrument(level="trace", skip(self, msg_entries), fields(msg_entries=%msg_entries.summary()))]
    async fn find_and_delete_conflict_logs(&mut self, msg_entries: &[Entry<C>]) -> Result<(), StorageError<C>> {
        // all msg_entries are inconsistent logs

        tracing::debug!(msg_entries=%msg_entries.summary(), "try to delete_inconsistent_log");

        let l = msg_entries.len();
        if l == 0 {
            return Ok(());
        }

        if let Some(last_log_id) = self.last_log_id {
            if msg_entries[0].log_id.index > last_log_id.index {
                return Ok(());
            }
        }

        tracing::debug!(
            "delete inconsistent log entries [{}, {}), last_log_id: {:?}, entries: {}",
            msg_entries[0].log_id,
            msg_entries[l - 1].log_id,
            self.last_log_id,
            msg_entries.summary()
        );

        self.delete_conflict_logs_since(msg_entries[0].log_id).await?;

        Ok(())
    }

    /// Append logs only when the first entry(prev_log_id) matches local store
    /// This way we keeps the log continuity.
    #[tracing::instrument(level="trace", skip(self, entries), fields(entries=%entries.summary()))]
    async fn append_apply_log_entries(
        &mut self,
        prev_log_id: Option<LogId<C>>,
        entries: &[Entry<C>],
        committed: Option<LogId<C>>,
    ) -> Result<AppendEntriesResponse<C>, StorageError<C>> {
        let mismatched = self.does_log_id_match(prev_log_id).await?;

        tracing::debug!(
            "check prev_log_id {:?} match: committed: {:?}, mismatched: {:?}",
            prev_log_id,
            self.committed,
            mismatched,
        );

        if let Some(mismatched_log_id) = mismatched {
            // prev_log_id mismatches, the logs [prev_log_id.index, +oo) are all inconsistent and should be removed
            if let Some(last_log_id) = self.last_log_id {
                if mismatched_log_id.index <= last_log_id.index {
                    tracing::debug!(%mismatched_log_id, "delete inconsistent log since prev_log_id");
                    self.delete_conflict_logs_since(mismatched_log_id).await?;
                }
            }

            return Ok(AppendEntriesResponse {
                vote: self.vote,
                success: false,
                conflict: true,
            });
        }

        // The entries left are all inconsistent log or absent
        let (n_matching, entries) = self.skip_matching_entries(entries).await?;

        tracing::debug!(
            ?self.committed,
            n_matching,
            entries = %entries.summary(),
            "skip matching entries",
        );

        // Before appending, if an entry overrides an inconsistent one, the entries after it must be deleted first.
        // Raft requires log ids are in total order by (term,index).
        // Otherwise the log id with max index makes committed entry invisible in election.
        self.find_and_delete_conflict_logs(entries).await?;

        self.append_log_entries(entries).await?;

        // commit index must not > last_log_id.index
        // This is guaranteed by caller.
        self.committed = committed;

        self.replicate_to_state_machine_if_needed().await?;

        self.report_metrics(Update::AsIs);

        Ok(AppendEntriesResponse {
            vote: self.vote,
            success: true,
            conflict: false,
        })
    }

    /// Returns number of entries that match local storage by comparing log_id,
    /// and the the unmatched entries.
    ///
    /// The entries in request that are matches local ones does not need to be append again.
    /// Filter them out.
    pub async fn skip_matching_entries<'s, 'e>(
        &'s mut self,
        entries: &'e [Entry<C>],
    ) -> Result<(usize, &'e [Entry<C>]), StorageError<C>> {
        let l = entries.len();

        for i in 0..l {
            let log_id = entries[i].log_id;

            if Some(log_id) <= self.committed {
                continue;
            }

            let index = log_id.index;

            // TODO(xp): this is a naive impl. Batch loading entries from storage.
            let log = self.storage.try_get_log_entry(index).await?;

            if let Some(local) = log {
                if local.log_id == log_id {
                    continue;
                }
            }

            return Ok((i, &entries[i..]));
        }

        Ok((l, &[]))
    }

    /// Return the mismatching log id if local store contains the log id.
    ///
    /// This way to check if the entries in append-entries request is consecutive with local logs.
    /// Raft only accept consecutive logs to be appended.
    pub async fn does_log_id_match(
        &mut self,
        remote_log_id: Option<LogId<C>>,
    ) -> Result<Option<LogId<C>>, StorageError<C>> {
        let log_id = match remote_log_id {
            None => {
                return Ok(None);
            }
            Some(x) => x,
        };

        // Committed entries are always safe and are consistent to a valid leader.
        if remote_log_id <= self.committed {
            return Ok(None);
        }

        let index = log_id.index;

        let log = self.storage.try_get_log_entry(index).await?;
        tracing::debug!(
            "check log id matching: local: {:?} remote: {}",
            log.as_ref().map(|x| x.log_id),
            log_id
        );

        if let Some(local) = log {
            if local.log_id == log_id {
                return Ok(None);
            }
        }

        Ok(Some(log_id))
    }

    /// Append the given entries to the log.
    ///
    /// Configuration changes are also detected and applied here. See `configuration changes`
    /// in the raft-essentials.md in this repo.
    #[tracing::instrument(level = "trace", skip(self, entries), fields(entries=%entries.summary()))]
    async fn append_log_entries(&mut self, entries: &[Entry<C>]) -> Result<(), StorageError<C>> {
        if entries.is_empty() {
            return Ok(());
        }

        // Check the given entries for any config changes and take the most recent.
        let last_conf_change = entries
            .iter()
            .filter_map(|ent| match &ent.payload {
                EntryPayload::Membership(conf) => Some(EffectiveMembership::new(ent.log_id, conf.clone())),
                _ => None,
            })
            .last();

        // TODO(xp): only when last_conf_change is newer than current one.
        //           For now it is guaranteed by `delete_logs()`, for it updates membership config when delete logs.
        //           and `skip_matching_entries()`, for it does not re-append existent log entries.
        //           This task should be done by StorageAdaptor.
        if let Some(conf) = last_conf_change {
            tracing::debug!({membership=?conf}, "applying new membership config received from leader");
            self.update_membership(conf);
        };

        // Replicate entries to log (same as append, but in follower mode).
        let entry_refs = entries.iter().collect::<Vec<_>>();
        self.storage.append_to_log(&entry_refs).await?;
        if let Some(entry) = entries.last() {
            self.last_log_id = Some(entry.log_id);
        }
        Ok(())
    }

    /// Replicate any outstanding entries to the state machine for which it is safe to do so.
    ///
    /// Very importantly, this routine must not block the main control loop main task, else it
    /// may cause the Raft leader to timeout the requests to this node.
    #[tracing::instrument(level = "trace", skip(self))]
    async fn replicate_to_state_machine_if_needed(&mut self) -> Result<(), StorageError<C>> {
        tracing::debug!(?self.last_applied, "replicate_to_sm_if_needed");

        // If we don't have any new entries to replicate, then do nothing.
        if self.committed <= self.last_applied {
            tracing::debug!(
                "committed({:?}) <= last_applied({:?}), return",
                self.committed,
                self.last_applied
            );
            return Ok(());
        }

        // Drain entries from the beginning of the cache up to commit index.

        let entries = self.storage.get_log_entries(self.last_applied.next_index()..self.committed.next_index()).await?;

        let last_log_id = entries.last().map(|x| x.log_id).unwrap();

        tracing::debug!("entries: {}", entries.as_slice().summary());
        tracing::debug!(?last_log_id);

        let entries_refs: Vec<_> = entries.iter().collect();

        apply_to_state_machine(&mut self.storage, &entries_refs, self.config.max_applied_log_to_keep).await?;

        self.last_applied = Some(last_log_id);

        self.report_metrics(Update::AsIs);
        self.trigger_log_compaction_if_needed(false).await;

        Ok(())
    }
}
