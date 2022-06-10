use std::sync::Arc;

use crate::core::apply_to_state_machine;
use crate::core::RaftCore;
use crate::core::ServerState;
use crate::error::AppendEntriesError;
use crate::membership::EffectiveMembership;
use crate::raft::AppendEntriesRequest;
use crate::raft::AppendEntriesResponse;
use crate::raft_types::LogIdOptionExt;
use crate::Entry;
use crate::EntryPayload;
use crate::LogId;
use crate::MembershipState;
use crate::MessageSummary;
use crate::RaftNetworkFactory;
use crate::RaftStorage;
use crate::RaftTypeConfig;
use crate::StorageError;

impl<C: RaftTypeConfig, N: RaftNetworkFactory<C>, S: RaftStorage<C>> RaftCore<C, N, S> {
    /// An RPC invoked by the leader to replicate log entries (§5.3); also used as heartbeat (§5.2).
    ///
    /// See `receiver implementation: AppendEntries RPC` in raft-essentials.md in this repo.
    #[tracing::instrument(level = "debug", skip(self, req))]
    pub(super) async fn handle_append_entries_request(
        &mut self,
        req: AppendEntriesRequest<C>,
    ) -> Result<AppendEntriesResponse<C::NodeId>, AppendEntriesError<C::NodeId>> {
        tracing::debug!(last_log_id=?self.engine.state.last_log_id, ?self.engine.state.last_applied, msg=%req.summary(), "handle_append_entries_request");

        let msg_entries = req.entries.as_slice();

        // Partial order compare: smaller than or incomparable
        if req.vote < self.engine.state.vote {
            tracing::debug!(?self.engine.state.vote, %req.vote, "AppendEntries RPC term is less than current term");
            return Ok(AppendEntriesResponse::HigherVote(self.engine.state.vote));
        }

        self.set_next_election_time();
        self.reject_election_for_a_while();

        tracing::debug!("start to check and update to latest term/leader");
        if req.vote > self.engine.state.vote {
            self.engine.state.vote = req.vote;
            self.save_vote().await?;

            // If not follower, become follower.
            if !self.engine.state.server_state.is_follower() && !self.engine.state.server_state.is_learner() {
                self.set_target_state(ServerState::Follower); // State update will emit metrics.
            }

            self.engine.metrics_flags.set_cluster_changed();
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
    pub(crate) async fn delete_conflict_logs_since(
        &mut self,
        start: LogId<C::NodeId>,
    ) -> Result<(), StorageError<C::NodeId>> {
        self.storage.delete_conflict_logs_since(start).await?;

        let st = &mut self.engine.state;
        st.last_log_id = self.storage.get_log_state().await?.last_log_id;

        // committed_membership, ... start, ... effective_membership // log
        //                                      last membership      // before delete start..
        // last membership                                           // after  delete start..

        let effective = st.membership_state.effective.clone();
        if Some(start.index) <= effective.log_id.index() {
            let committed = st.membership_state.committed.clone();

            assert!(
                committed.log_id < Some(start),
                "committed membership can not conflict with the leader"
            );

            let mem_state = MembershipState {
                committed: committed.clone(),
                effective: committed,
            };
            self.update_membership(mem_state);
            tracing::debug!("Done update membership");
        }

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
    async fn find_and_delete_conflict_logs(&mut self, msg_entries: &[Entry<C>]) -> Result<(), StorageError<C::NodeId>> {
        // all msg_entries are inconsistent logs

        tracing::debug!(msg_entries=%msg_entries.summary(), "try to delete_inconsistent_log");

        let l = msg_entries.len();
        if l == 0 {
            return Ok(());
        }

        if let Some(last_log_id) = self.engine.state.last_log_id {
            if msg_entries[0].log_id.index > last_log_id.index {
                return Ok(());
            }
        }

        tracing::debug!(
            "delete inconsistent log entries [{}, {}), last_log_id: {:?}, entries: {}",
            msg_entries[0].log_id,
            msg_entries[l - 1].log_id,
            self.engine.state.last_log_id,
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
        prev_log_id: Option<LogId<C::NodeId>>,
        entries: &[Entry<C>],
        committed: Option<LogId<C::NodeId>>,
    ) -> Result<AppendEntriesResponse<C::NodeId>, StorageError<C::NodeId>> {
        let mismatched = self.does_log_id_match(prev_log_id).await?;

        tracing::debug!(
            "check prev_log_id {:?} match: committed: {:?}, mismatched: {:?}",
            prev_log_id,
            self.engine.state.committed,
            mismatched,
        );

        if let Some(mismatched_log_id) = mismatched {
            // prev_log_id mismatches, the logs [prev_log_id.index, +oo) are all inconsistent and should be removed
            if let Some(last_log_id) = self.engine.state.last_log_id {
                if mismatched_log_id.index <= last_log_id.index {
                    tracing::debug!(%mismatched_log_id, "delete inconsistent log since prev_log_id");
                    self.delete_conflict_logs_since(mismatched_log_id).await?;
                }
            }

            return Ok(AppendEntriesResponse::Conflict);
        }

        // The entries left are all inconsistent log or absent
        let (n_matching, entries) = self.skip_matching_entries(entries).await?;

        tracing::debug!(
            ?self.engine.state.committed,
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
        self.engine.state.committed = committed;
        {
            let st = &mut self.engine.state;
            if st.committed >= st.membership_state.effective.log_id {
                st.membership_state.committed = st.membership_state.effective.clone();
            }
        }

        self.replicate_to_state_machine_if_needed().await?;

        Ok(AppendEntriesResponse::Success)
    }

    /// Returns number of entries that match local storage by comparing log_id,
    /// and the the unmatched entries.
    ///
    /// The entries in request that are matches local ones does not need to be append again.
    /// Filter them out.
    pub async fn skip_matching_entries<'s, 'e>(
        &'s mut self,
        entries: &'e [Entry<C>],
    ) -> Result<(usize, &'e [Entry<C>]), StorageError<C::NodeId>> {
        let l = entries.len();

        for i in 0..l {
            let log_id = entries[i].log_id;

            if Some(log_id) <= self.engine.state.committed {
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
        remote_log_id: Option<LogId<C::NodeId>>,
    ) -> Result<Option<LogId<C::NodeId>>, StorageError<C::NodeId>> {
        let log_id = match remote_log_id {
            None => {
                return Ok(None);
            }
            Some(x) => x,
        };

        // Committed entries are always safe and are consistent to a valid leader.
        if remote_log_id <= self.engine.state.committed {
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
    /// TODO(xp): this method is only called by a follower or learner.
    #[tracing::instrument(level = "trace", skip(self, entries), fields(entries=%entries.summary()))]
    async fn append_log_entries(&mut self, entries: &[Entry<C>]) -> Result<(), StorageError<C::NodeId>> {
        if entries.is_empty() {
            return Ok(());
        }

        // Find at most the last two membership log entries.
        // The older is committed, the newer is effective.
        let mut memberships = vec![];
        for ent in entries.iter().rev() {
            if let EntryPayload::Membership(conf) = &ent.payload {
                memberships.insert(0, EffectiveMembership::new(Some(ent.log_id), conf.clone()));
                if memberships.len() == 2 {
                    break;
                }
            };
        }

        if !memberships.is_empty() {
            tracing::debug!(memberships=?memberships, "applying new membership configs received from leader");

            let st = &self.engine.state;
            let new_mem_state = if memberships.len() == 1 {
                MembershipState {
                    committed: st.membership_state.effective.clone(),
                    effective: Arc::new(memberships[0].clone()),
                }
            } else {
                // len() == 2
                MembershipState {
                    committed: Arc::new(memberships[0].clone()),
                    effective: Arc::new(memberships[1].clone()),
                }
            };

            self.update_membership(new_mem_state);
        };

        // Replicate entries to log (same as append, but in follower mode).
        let entry_refs = entries.iter().collect::<Vec<_>>();
        self.storage.append_to_log(&entry_refs).await?;
        if let Some(entry) = entries.last() {
            self.engine.state.last_log_id = Some(entry.log_id);
        }
        Ok(())
    }

    /// Replicate any outstanding entries to the state machine for which it is safe to do so.
    ///
    /// Very importantly, this routine must not block the main control loop main task, else it
    /// may cause the Raft leader to timeout the requests to this node.
    #[tracing::instrument(level = "debug", skip(self))]
    pub(crate) async fn replicate_to_state_machine_if_needed(&mut self) -> Result<(), StorageError<C::NodeId>> {
        tracing::debug!(?self.engine.state.last_applied, ?self.engine.state.committed, "replicate_to_sm_if_needed");

        // If we don't have any new entries to replicate, then do nothing.
        if self.engine.state.committed <= self.engine.state.last_applied {
            tracing::debug!(
                "committed({:?}) <= last_applied({:?}), return",
                self.engine.state.committed,
                self.engine.state.last_applied
            );
            // TODO(xp): this should be moved to upper level.
            self.engine.metrics_flags.set_data_changed();
            return Ok(());
        }

        // Drain entries from the beginning of the cache up to commit index.

        let entries = self
            .storage
            .get_log_entries(self.engine.state.last_applied.next_index()..self.engine.state.committed.next_index())
            .await?;

        let last_log_id = entries.last().map(|x| x.log_id).unwrap();

        tracing::debug!("entries: {}", entries.as_slice().summary());
        tracing::debug!(?last_log_id);

        let entries_refs: Vec<_> = entries.iter().collect();

        apply_to_state_machine(self, &entries_refs, self.config.max_applied_log_to_keep).await?;

        self.trigger_log_compaction_if_needed(false).await;
        self.engine.metrics_flags.set_data_changed();
        Ok(())
    }
}
