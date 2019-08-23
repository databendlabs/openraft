use std::sync::Arc;

use actix::prelude::*;

use crate::{
    AppData, AppError,
    common::{ApplyLogsTask, DependencyAddr, UpdateCurrentLeader},
    network::RaftNetwork,
    messages::{AppendEntriesRequest, AppendEntriesResponse, ConflictOpt, Entry, EntryType},
    raft::{RaftState, Raft, SnapshotState},
    storage::{GetLogEntries, RaftStorage, ReplicateLogEntries},
};

impl<D: AppData, E: AppError, N: RaftNetwork<D>, S: RaftStorage<D, E>> Handler<AppendEntriesRequest<D>> for Raft<D, E, N, S> {
    type Result = ResponseActFuture<Self, AppendEntriesResponse, ()>;

    /// An RPC invoked by the leader to replicate log entries (§5.3); also used as heartbeat (§5.2).
    ///
    /// This method implements the append entries algorithm and upholds all of the safety checks
    /// detailed in §5.3.
    ///
    /// Implementation overview from spec:
    ///
    /// 1. Reply `false` if `term` is less than node's current `term` (§5.1).
    /// 2. Reply `false` if log doesn’t contain an entry at `prev_log_index` whose term
    ///    matches `prev_log_term` (§5.3).
    /// 3. If an existing entry conflicts with a new one (same index but different terms), delete the
    ///    existing entry and all that follow it (§5.3).
    /// 4. Append any new entries not already in the log.
    /// 5. If `leader_commit` is greater than node's commit index, set nodes commit index to
    ///    `min(leader_commit, index of last new entry)`.
    ///
    /// The essential goal of this algorithm is that the receiver (the node on which this method
    /// is being executed) must find the exact entry in its log specified by the RPC's last index
    /// and last term fields, and then begin writing the new entries thereafter.
    ///
    /// When the receiver can not find the entry specified in the RPC's prev index & prev term
    /// fields, it will respond with a failure to the leader. **This implementation of Raft
    /// includes the _conflicting term_ optimization** which is intended to reduce the number of
    /// rejected append entries RPCs from followers which are lagging behind, which is detailed in
    /// §5.3. In such cases, if the Raft cluster is configured with a snapshot policy other than
    /// `Disabled`, the leader will make a determination if an `InstallSnapshot` RPC should be
    /// sent to this node.
    ///
    /// In Raft, the leader handles inconsistencies by forcing the followers’ logs to duplicate
    /// its own. This means that conflicting entries in follower logs will be overwritten with
    /// entries from the leader’s log. §5.4 details the safety of this protocol. It is important
    /// to note that logs which are _committed_ will not be overwritten. This is a critical
    /// feature of Raft.
    ///
    /// Raft also gurantees that only logs which have been comitted may be applied to the state
    /// machine, which ensures that there will never be a case where a log needs to be reverted
    /// after being applied to the state machine.
    ///
    /// #### inconsistency example
    /// Followers may receive valid append entries requests from leaders, append them, respond,
    /// and before the leader is able to replicate the entries to a majority of nodes, the leader
    /// may die, a new leader may be elected which does not have the same entries, as they were
    /// not replicated to a majority of followers, and the new leader will proceeed to overwrite
    /// the inconsistent entries.
    fn handle(&mut self, msg: AppendEntriesRequest<D>, ctx: &mut Self::Context) -> Self::Result {
        // Only handle requests if actor has finished initialization.
        if let &RaftState::Initializing = &self.state {
            return Box::new(fut::err(()));
        }

        // If message's term is less than most recent term, then we do not honor the request.
        if &msg.term < &self.current_term {
            return Box::new(fut::ok(AppendEntriesResponse{term: self.current_term, success: false, conflict_opt: None}));
        }

        // Update election timeout.
        self.update_election_timeout(ctx);

        // Update current term if needed.
        if self.current_term != msg.term {
            self.update_current_term(msg.term, None);
            self.save_hard_state(ctx);
        }

        // Update current leader if needed.
        if self.current_leader != Some(msg.leader_id) {
            self.update_current_leader(ctx, UpdateCurrentLeader::OtherNode(msg.leader_id));
        }

        // Transition to follower state if needed.
        match &mut self.state {
            // Ensure we are not in a snapshotting state.
            RaftState::Follower(inner) => match inner.snapshot_state {
                SnapshotState::Idle => (),
                _ => inner.snapshot_state = SnapshotState::Idle,
            }
            // NonVoters stay in this state until a config change is received which changes its state.
            RaftState::NonVoter => (),
            // Any other state needs to transition to follower.
            _ => self.become_follower(ctx),
        }

        // Kick off process of applying logs to state machine based on `msg.leader_commit`.
        self.commit_index = msg.leader_commit; // The value for `self.commit_index` is only updated here when not the leader.
        if &self.commit_index > &self.last_applied {
            let _ = self.apply_logs_pipeline.unbounded_send(ApplyLogsTask::Outstanding);
        }

        // If this is just a heartbeat, then respond.
        if msg.entries.len() == 0 {
            return Box::new(fut::ok(AppendEntriesResponse{term: self.current_term, success: true, conflict_opt: None}));
        }

        // If RPC's `prev_log_index` is 0, or the RPC's previous log info matches the local
        // log info, then replication is g2g.
        let (term, msg_prev_index, msg_prev_term) = (self.current_term, msg.prev_log_index, msg.prev_log_term);
        let has_prev_log_match = &msg.prev_log_index == &u64::min_value() || (&msg_prev_index == &self.last_log_index && &msg_prev_term == &self.last_log_term);
        if has_prev_log_match {
            return Box::new(self.append_log_entries(ctx, Arc::new(msg.entries))
                .map(move |_, _, _| {
                    AppendEntriesResponse{term, success: true, conflict_opt: None}
                }));
        }

        // Previous log info doesn't immediately line up, so perform log consistency check and
        // proceed based on its result.
        Box::new(self.log_consistency_check(ctx, msg_prev_index, msg_prev_term)
            .and_then(move |res, act, ctx| match res {
                Some(conflict_opt) => {
                    fut::Either::A(fut::ok(
                        AppendEntriesResponse{term, success: false, conflict_opt: Some(conflict_opt)}
                    ))
                }
                None => {
                    fut::Either::B(act.append_log_entries(ctx, Arc::new(msg.entries))
                        .map(move |_, _, _| {
                            AppendEntriesResponse{term, success: true, conflict_opt: None}
                        }))
                }
            }))
    }
}

impl<D: AppData, E: AppError, N: RaftNetwork<D>, S: RaftStorage<D, E>> Raft<D, E, N, S> {
    /// Append the given entries to the log.
    ///
    /// This routine also encapsulates all logic which must be performed related to appending log
    /// entries.
    ///
    /// One important piece of logic to note here is the handling of config change entries. Per
    /// the Raft spec in §6:
    ///
    /// > Once a given server adds the new configuration entry to its log, it uses that
    /// > configuration for all future decisions (a server always uses the latest configuration in
    /// > its log, regardless of whether the entry is committed).
    ///
    /// This routine will extract the most recent (the latter most) entry in the given payload of
    /// entries which is a config change entry and will update the node's member state based on
    /// that entry.
    fn append_log_entries(
        &mut self, ctx: &mut Context<Self>, entries: Arc<Vec<Entry<D>>>,
    ) -> impl ActorFuture<Actor=Self, Item=(), Error=()> {
        // If we are already eppending entries, then abort this operation.
        if self.is_appending_logs {
            return fut::Either::A(fut::err(()));
        }

        // Check the given entries for any config changes and take the most recent.
        let last_conf_change = entries.iter().filter_map(|ent| match &ent.entry_type {
            EntryType::ConfigChange(conf) => Some(conf),
            _ => None,
        }).last();
        let f = match last_conf_change {
            Some(conf) => {
                // Update membership info & apply hard state.
                fut::Either::A(self.update_membership(ctx, conf.membership.clone()))
            }
            None => fut::Either::B(fut::ok(())),
        };

        fut::Either::B(f.and_then(move |_, act, _| {
            act.is_appending_logs = true;
            fut::wrap_future(act.storage.send(ReplicateLogEntries::new(entries.clone())))
                .map_err(|err, act: &mut Self, ctx| act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftStorage))
                .and_then(|res, act, ctx| act.map_fatal_storage_result(ctx, res))
                .map(move |_, act, _| {
                    if let Some((idx, term)) = entries.last().map(|elem| (elem.index, elem.term)) {
                        act.last_log_index = idx;
                        act.last_log_term = term;
                    }
                })
                .then(|res, act, _| {
                    act.is_appending_logs = false;
                    fut::result(res)
                })
        }))
    }

    /// Perform the AppendEntries RPC consistency check.
    ///
    /// If the log entry at the specified index does not exist, the most recent entry in the log
    /// will be used to build and return a `ConflictOpt` struct to be sent back to the leader.
    ///
    /// If The log entry at the specified index does exist, but the terms to no match up, this
    /// implementation will fetch the last 50 entries from the given index, and will use the
    /// earliest entry from the log which is still in the given term to build a `ConflictOpt`
    /// struct to be sent back to the leader.
    ///
    /// If everyhing checks out, a `None` value will be returned and log replication may continue.
    fn log_consistency_check(
        &mut self, _: &mut Context<Self>, index: u64, term: u64,
    ) -> impl ActorFuture<Actor=Self, Item=Option<ConflictOpt>, Error=()> {
        let storage = self.storage.clone();
        fut::wrap_future(self.storage.send(GetLogEntries::new(index, index)))
            .map_err(|err, act: &mut Self, ctx| act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftStorage))
            .and_then(|res, act, ctx| act.map_fatal_storage_result(ctx, res))
            .and_then(move |res, act, _| {
                match res.last() {
                    // The target entry was not found. This can only mean that we don't have the
                    // specified index yet. Use the last known index & term.
                    None => {
                        fut::Either::A(fut::ok(Some(ConflictOpt{term: act.last_log_term, index: act.last_log_index})))
                    }
                    // The target entry was found. Compare its term with target term to ensure
                    // everything is consistent.
                    Some(target_entry) => {
                        let (target_entry_index, target_entry_term) = (target_entry.index, target_entry.term);
                        if &target_entry_index == &index && &target_entry_term == &term {
                            // Everything checks out. We're g2g.
                            fut::Either::A(fut::ok(None))
                        } else {
                            // Logs are inconsistent. Fetch the last 50 logs, and use the last
                            // entry of that payload which is still in the target term for
                            // conflict optimization.
                            let start = if index >= 50 { index - 50 } else { 0 };
                            fut::Either::B(fut::wrap_future(storage.send(GetLogEntries::new(start, index)))
                                .map_err(|err, act: &mut Self, ctx| act.map_fatal_actix_messaging_error(ctx, err, DependencyAddr::RaftStorage))
                                .and_then(|res, act, ctx| act.map_fatal_storage_result(ctx, res))
                                .and_then(move |res, act, _| {
                                    match res.iter().find(|entry| entry.term == term) {
                                        Some(entry) => {
                                            fut::ok(Some(ConflictOpt{
                                                term: entry.term,
                                                index: entry.index,
                                            }))
                                        }
                                        None => {
                                            fut::ok(Some(ConflictOpt{
                                                term: act.last_log_term,
                                                index: act.last_log_index,
                                            }))
                                        }
                                    }
                                }))
                        }
                    }
                }
            })
    }
}
